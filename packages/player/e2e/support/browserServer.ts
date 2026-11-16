// Playwright's browser server in the pinned image, shared by the player's effect suite and the app's e2e suite.
// `SLIDOCK_E2E_FIREFOX=docker` and `SLIDOCK_E2E_WEBKIT=docker` add a project whose browser runs there (`firefox`,
// `webkit-docker`): the tests stay on this machine, the browser reaches this machine's loopback through the connection
// (`exposeNetwork`), and reads `file://` pages from the mounted paths, mounted read-only at the same path. One server
// serves both browsers; the global setup starts it and stops it after. Each run's container carries the run's own
// name and label, and a run removes only containers with its label: another run's server, live or left over, is
// never touched, and a port it holds stops this run with its name.
import { devices } from '@playwright/test';
import type { PlaywrightTestConfig } from '@playwright/test';
import { spawnSync } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { createRequire } from 'node:module';
import { dirname, join } from 'node:path';

export const PLAYWRIGHT_IMAGE = 'mcr.microsoft.com/playwright:v1.60.0-noble';

export const FIREFOX_IN_DOCKER = process.env.SLIDOCK_E2E_FIREFOX === 'docker';
export const WEBKIT_IN_DOCKER = process.env.SLIDOCK_E2E_WEBKIT === 'docker';

/** A port from `name` in the environment, else `fallback`: parallel runs pick their own. */
export function envPort(name: string, fallback: number): number {
  const value = Number(process.env[name]);
  return Number.isInteger(value) && value > 0 && value < 65536 ? value : fallback;
}

/** The player suite's browser server port (`SLIDOCK_E2E_PLAYER_BROWSER_PORT`); the app's suite has its own. */
export const BROWSER_SERVER_PORT = envPort('SLIDOCK_E2E_PLAYER_BROWSER_PORT', 3556);

type Project = NonNullable<PlaywrightTestConfig['projects']>[number];
type Use = NonNullable<Project['use']>;

/** The projects whose browser runs in the image, each given `use` on top of its device. */
export function dockerProjects(port: number, use: Use = {}): Project[] {
  const connectOptions = { wsEndpoint: `ws://127.0.0.1:${port}/`, exposeNetwork: '<loopback>' };
  return [
    ...(FIREFOX_IN_DOCKER ? [{ name: 'firefox', use: { ...devices['Desktop Firefox'], ...use, connectOptions } }] : []),
    ...(WEBKIT_IN_DOCKER
      ? [{ name: 'webkit-docker', use: { ...devices['Desktop Safari'], ...use, connectOptions } }]
      : []),
  ];
}

export interface ServerOptions {
  port: number;
  /** Host paths the browser opens and the server runs from, mounted read-only at the same path. */
  mounts: readonly string[];
  /** Font files the container's fontconfig indexes beside the image's own, as `[host path, file name]`. */
  fonts?: readonly (readonly [string, string])[];
}

/** One `docker` command's outcome. */
export interface DockerResult {
  status: number | null;
  stdout: string;
  stderr: string;
}

/** Runs `docker` with these arguments. */
export type Docker = (args: readonly string[]) => DockerResult;

/** What the server's start depends on, replaced in its tests. */
export interface Runtime {
  docker: Docker;
  sleep: (ms: number) => Promise<void>;
  /** This run's id: the process id and a random suffix. */
  runId: string;
}

/** The label each run's container carries, valued with the run's id. */
export const OWNER_LABEL = 'dev.slidock.e2e.browser-server';

const realDocker: Docker = (args) => {
  const result = spawnSync('docker', args, { encoding: 'utf8' });
  return { status: result.status, stdout: result.stdout ?? '', stderr: result.stderr ?? `${result.error ?? ''}` };
};

export function defaultRuntime(): Runtime {
  return {
    docker: realDocker,
    sleep: (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
    runId: `${process.pid}-${randomBytes(4).toString('hex')}`,
  };
}

/** The run's container name: the port, then the run's id, so two runs never share one. */
export function containerName(port: number, runId: string): string {
  return `slidock-e2e-browsers-${port}-${runId}`;
}

/** Removes every container labelled as this run's, and only those. */
export function removeOwnContainers(docker: Docker, runId: string): void {
  const listed = docker(['ps', '--all', '--quiet', '--filter', `label=${OWNER_LABEL}=${runId}`]);
  const ids = listed.stdout.split(/\s+/).filter((id) => /^[0-9a-f]{12,64}$/.test(id));
  if (ids.length > 0) {
    docker(['rm', '--force', ...ids]);
  }
}

/** The names of the running containers that publish `port`. */
function holdersOf(docker: Docker, port: number): string[] {
  const listed = docker(['ps', '--filter', `publish=${port}`, '--format', '{{.Names}}']);
  return listed.stdout.split('\n').filter((name) => name.trim() !== '');
}

/**
 * Starts the browser server when a project needs it, on this machine's loopback; returns its teardown (a global
 * setup's return value), or undefined when no project runs in Docker.
 */
export async function startBrowserServer(
  options: ServerOptions,
  runtime: Runtime = defaultRuntime(),
): Promise<(() => void) | undefined> {
  if (!FIREFOX_IN_DOCKER && !WEBKIT_IN_DOCKER) {
    return undefined;
  }
  // The playwright-core this run uses (@playwright/test → playwright → playwright-core), so the server and the
  // client match.
  const resolveFrom = (from: string, name: string) => createRequire(from).resolve(name);
  const playwright = resolveFrom(resolveFrom(import.meta.url, '@playwright/test'), 'playwright');
  const core = dirname(resolveFrom(playwright, 'playwright-core/package.json'));
  return runBrowserServer(options, join(core, 'cli.js'), runtime);
}

/** Runs `cli` (playwright-core's) as the server in a container of this run's own; see {@link startBrowserServer}. */
export async function runBrowserServer(options: ServerOptions, cli: string, runtime: Runtime): Promise<() => void> {
  const { docker, runId } = runtime;
  const container = containerName(options.port, runId);
  // The mounts hold the playwright-core too: it sits in the repository's node_modules.
  const mounts = options.mounts.flatMap((path) => ['--mount', `type=bind,src=${path},dst=${path},readonly`]);
  const fonts = (options.fonts ?? []).flatMap(([path, name]) => [
    '--mount',
    `type=bind,src=${path},dst=/usr/share/fonts/slidock-e2e/${name},readonly`,
  ]);
  const stop = () => removeOwnContainers(docker, runId);
  const started = docker([
    'run',
    '--detach',
    '--rm',
    '--init',
    '--name',
    container,
    '--label',
    `${OWNER_LABEL}=${runId}`,
    '--publish',
    `127.0.0.1:${options.port}:${options.port}`,
    ...mounts,
    ...fonts,
    PLAYWRIGHT_IMAGE,
    'sh',
    '-c',
    'fc-cache -f > /dev/null 2>&1; exec node "$0" run-server --port "$1" --host 0.0.0.0',
    cli,
    String(options.port),
  ]);
  if (started.status !== 0) {
    stop();
    const holders = holdersOf(docker, options.port);
    throw new Error(
      holders.length > 0
        ? `the browser server's port ${options.port} is held by ${holders.join(', ')}, which this run did not ` +
            'start and leaves running: give this run another port (SLIDOCK_E2E_BROWSER_PORT, ' +
            'SLIDOCK_E2E_PLAYER_BROWSER_PORT or SLIDOCK_BENCH_E2E_BROWSER_PORT), or remove it if no run owns it'
        : `docker run ${PLAYWRIGHT_IMAGE} failed: ${started.stderr.trim()}`,
    );
  }
  for (let attempt = 0; attempt < 120; attempt++) {
    const logs = docker(['logs', container]);
    if (`${logs.stdout}${logs.stderr}`.includes('Listening on')) {
      return stop;
    }
    await runtime.sleep(500);
  }
  stop();
  throw new Error(`the browser server in ${PLAYWRIGHT_IMAGE} did not start`);
}
