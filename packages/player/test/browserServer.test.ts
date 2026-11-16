import { describe, expect, it } from 'vitest';

import {
  OWNER_LABEL,
  containerName,
  defaultRuntime,
  removeOwnContainers,
  runBrowserServer,
} from '../e2e/support/browserServer';
import type { Docker, DockerResult, Runtime } from '../e2e/support/browserServer';

interface Container {
  id: string;
  name: string;
  labels: Map<string, string>;
  port: number | null;
  running: boolean;
}

/** A Docker daemon modelled as far as the browser server uses it: run, ps, logs and rm. */
class FakeDocker {
  containers: Container[] = [];
  commands: string[][] = [];
  /** Whether a started server prints `Listening on`. */
  listens = true;
  #next = 0;

  add(name: string, options: { labels?: Record<string, string>; port?: number } = {}): Container {
    const container: Container = {
      id: (0xabc000000000 + this.#next++).toString(16),
      name,
      labels: new Map(Object.entries(options.labels ?? {})),
      port: options.port ?? null,
      running: true,
    };
    this.containers.push(container);
    return container;
  }

  names(): string[] {
    return this.containers.map((c) => c.name);
  }

  readonly docker: Docker = (args) => {
    this.commands.push([...args]);
    const ok = (stdout = ''): DockerResult => ({ status: 0, stdout, stderr: '' });
    const fail = (stderr: string): DockerResult => ({ status: 125, stdout: '', stderr });
    const value = (flag: string) => args.filter((_, i) => args[i - 1] === flag);
    switch (args[0]) {
      case 'run': {
        const [name = ''] = value('--name');
        const [publish = ''] = value('--publish');
        const port = Number(publish.split(':').at(-1));
        if (this.containers.some((c) => c.name === name)) {
          return fail(`Conflict. The container name "/${name}" is already in use`);
        }
        const labels = Object.fromEntries(value('--label').map((l) => l.split('=') as [string, string]));
        const held = this.containers.some((c) => c.running && c.port === port);
        const container = this.add(name, { labels, port });
        if (held) {
          container.running = false;
          return fail(`Bind for 127.0.0.1:${port} failed: port is already allocated`);
        }
        return ok(`${container.id}\n`);
      }
      case 'ps': {
        let listed = this.containers.filter((c) => args.includes('--all') || c.running);
        for (const filter of value('--filter')) {
          const [kind, rest = ''] = filter.split(/=(.*)/s);
          if (kind === 'label') {
            const [key, wanted] = rest.split(/=(.*)/s);
            listed = listed.filter(
              (c) => c.labels.has(key ?? '') && (wanted === undefined || c.labels.get(key ?? '') === wanted),
            );
          } else if (kind === 'publish') {
            listed = listed.filter((c) => c.port === Number(rest));
          }
        }
        const format = value('--format')[0];
        return ok(listed.map((c) => (format === '{{.Names}}' ? c.name : c.id)).join('\n'));
      }
      case 'logs': {
        const container = this.containers.find((c) => c.name === args[1] || c.id === args[1]);
        return container ? ok(this.listens ? 'Listening on ws://0.0.0.0/' : '') : fail('No such container');
      }
      case 'rm': {
        const ids = args.slice(1).filter((a) => !a.startsWith('-'));
        this.containers = this.containers.filter((c) => !ids.includes(c.id) && !ids.includes(c.name));
        return ok();
      }
      default:
        return fail(`unknown command ${args[0]}`);
    }
  };

  runtime(runId: string): Runtime {
    return { docker: this.docker, sleep: async () => {}, runId };
  }
}

const options = (port: number) => ({ port, mounts: ['/repo'] });
const CLI = '/repo/node_modules/playwright-core/cli.js';

describe('the e2e browser server', () => {
  it('leaves a live container it did not start running, whatever its name', async () => {
    const fake = new FakeDocker();
    // The name every run used to share, and another run's server on its own port.
    fake.add('slidock-e2e-browsers-3555');
    fake.add(containerName(3600, '999-aaaaaaaa'), { labels: { [OWNER_LABEL]: '999-aaaaaaaa' }, port: 3600 });
    const stop = await runBrowserServer(options(3555), CLI, fake.runtime('1-bbbbbbbb'));
    expect(fake.names()).toEqual([
      'slidock-e2e-browsers-3555',
      'slidock-e2e-browsers-3600-999-aaaaaaaa',
      'slidock-e2e-browsers-3555-1-bbbbbbbb',
    ]);
    stop();
    expect(fake.names()).toEqual(['slidock-e2e-browsers-3555', 'slidock-e2e-browsers-3600-999-aaaaaaaa']);
    // It removed nothing but by its own label.
    const removals = fake.commands.filter((c) => c[0] === 'rm' || c[0] === 'stop' || c[0] === 'kill');
    expect(removals).toHaveLength(1);
    expect(fake.commands).toContainEqual(['ps', '--all', '--quiet', '--filter', `label=${OWNER_LABEL}=1-bbbbbbbb`]);
  });

  it('runs two servers at once, each removing only its own', async () => {
    const fake = new FakeDocker();
    const stopA = await runBrowserServer(options(3555), CLI, fake.runtime('10-aaaaaaaa'));
    const stopB = await runBrowserServer(options(3556), CLI, fake.runtime('11-bbbbbbbb'));
    expect(fake.containers.filter((c) => c.running)).toHaveLength(2);
    stopA();
    expect(fake.names()).toEqual(['slidock-e2e-browsers-3556-11-bbbbbbbb']);
    stopB();
    expect(fake.names()).toEqual([]);
  });

  it('stops on a port another run holds, names the holder and leaves it running', async () => {
    const fake = new FakeDocker();
    const stopA = await runBrowserServer(options(3555), CLI, fake.runtime('10-aaaaaaaa'));
    await expect(runBrowserServer(options(3555), CLI, fake.runtime('12-cccccccc'))).rejects.toThrow(
      /port 3555 is held by slidock-e2e-browsers-3555-10-aaaaaaaa, which this run did not start and leaves running/,
    );
    // The second run's own container, created but never started, is gone; the first run's is untouched.
    expect(fake.names()).toEqual(['slidock-e2e-browsers-3555-10-aaaaaaaa']);
    expect(fake.containers[0]?.running).toBe(true);
    stopA();
  });

  it('removes its own container when the server never listens', async () => {
    const fake = new FakeDocker();
    fake.add('slidock-e2e-browsers-3555');
    fake.listens = false;
    await expect(runBrowserServer(options(3555), CLI, fake.runtime('13-dddddddd'))).rejects.toThrow(/did not start/);
    expect(fake.names()).toEqual(['slidock-e2e-browsers-3555']);
  });

  it('removes only well-formed ids that its label lists', () => {
    const commands: string[][] = [];
    const docker: Docker = (args) => {
      commands.push([...args]);
      return { status: 0, stdout: args[0] === 'ps' ? 'abc123abc123\n\n--all\nnot-an-id\n' : '', stderr: '' };
    };
    removeOwnContainers(docker, '5-eeeeeeee');
    expect(commands.at(-1)).toEqual(['rm', '--force', 'abc123abc123']);
    const none: Docker = (args) => {
      commands.push([...args]);
      return { status: 0, stdout: '', stderr: '' };
    };
    commands.length = 0;
    removeOwnContainers(none, '5-eeeeeeee');
    expect(commands.map((c) => c[0])).toEqual(['ps']);
  });

  it("names each run's container after its port, process and a random suffix", () => {
    const a = defaultRuntime().runId;
    const b = defaultRuntime().runId;
    expect(a).toMatch(new RegExp(`^${process.pid}-[0-9a-f]{8}$`));
    expect(a).not.toBe(b);
    expect(containerName(3555, a)).toBe(`slidock-e2e-browsers-3555-${a}`);
  });
});
