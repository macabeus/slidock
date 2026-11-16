// The shell lane's watch on a deck opened as a user opens it, evaluated before the page's own scripts. It changes
// nothing of how the page boots: the deck's own script mounts its player under the deck's Content Security Policy, or
// does not. The watch notes whether it did (passing the call through and keeping the player it returns), what the
// page reports (policy violations, uncaught errors), and every animation the player starts, so `settle` can wait for
// the first slide to come to rest. Media never play (the static lane's render policy).
(() => {
  const watch = { player: null, mounted: false, body: false, errors: [], animations: [] };
  window.__sdShell = watch;
  document.addEventListener('securitypolicyviolation', (event) => {
    watch.errors.push(`csp ${event.effectiveDirective} ${event.blockedURI || 'inline'}`);
  });
  window.addEventListener('error', (event) => {
    watch.errors.push(`error ${event.message}`);
  });
  window.addEventListener('unhandledrejection', (event) => {
    watch.errors.push(`rejection ${String(event.reason)}`);
  });
  HTMLMediaElement.prototype.play = function play() {
    return Promise.resolve();
  };
  const realAnimate = Element.prototype.animate;
  Element.prototype.animate = function animate(keyframes, options) {
    const animation = realAnimate.call(this, keyframes, options);
    watch.animations.push(animation);
    return animation;
  };
  let slidock;
  Object.defineProperty(window, 'Slidock', {
    configurable: true,
    get: () => slidock,
    set(value) {
      slidock =
        value !== null && typeof value === 'object' && typeof value.mount === 'function'
          ? Object.create(value, {
              mount: {
                value(host, ...rest) {
                  const player = value.mount.call(value, host, ...rest);
                  watch.mounted = true;
                  watch.body = host === document.body;
                  watch.player = player;
                  return player;
                },
              },
            })
          : value;
    },
  });

  const macrotask = () => new Promise((resolve) => setTimeout(resolve, 0));
  const frame = () => new Promise((resolve) => requestAnimationFrame(() => resolve()));
  const finite = (animation) => {
    const end = animation.effect ? animation.effect.getComputedTiming().endTime : 0;
    return Number.isFinite(end);
  };

  /** Waits (at most `ms`) until the mounted player's slide is ready and every animation that can end has ended. */
  watch.settle = async (ms) => {
    const until = performance.now() + ms;
    if (watch.player !== null) {
      await document.fonts.ready;
      let seen = -1;
      while (performance.now() < until && seen !== watch.animations.length) {
        seen = watch.animations.length;
        const running = watch.animations.filter(
          (a) => finite(a) && a.playState !== 'finished' && a.playState !== 'idle',
        );
        await Promise.race([
          Promise.all(running.map((a) => a.finished.catch(() => undefined))),
          new Promise((resolve) => setTimeout(resolve, Math.max(0, until - performance.now()))),
        ]);
        await macrotask();
        await macrotask();
        await frame();
      }
      for (const layer of document.querySelectorAll('.sd-layer')) {
        const root = layer.shadowRoot;
        if (root !== null) {
          await Promise.all(Array.from(root.querySelectorAll('img'), (image) => image.decode().catch(() => undefined)));
        }
      }
      await frame();
      await frame();
    }
    // The player's own chrome around and beside its stage: the page's pixels the stage does not wholly cover (a deck
    // whose size is not whole pixels letterboxes by a fraction), and the slide counter, which shows for a while after
    // each move.
    const chrome = [];
    let aligned = true;
    const stage = document.querySelector('.sd-player .sd-frame');
    if (stage !== null) {
      const box = stage.getBoundingClientRect();
      aligned = [box.x, box.y, box.width, box.height].every((v) => Math.abs(v - Math.round(v)) < 1e-6);
      const [left, top] = [Math.ceil(box.x), Math.ceil(box.y)];
      const [right, bottom] = [Math.floor(box.x + box.width), Math.floor(box.y + box.height)];
      const [width, height] = [innerWidth, innerHeight];
      for (const strip of [
        [0, 0, width, top],
        [0, bottom, width, height - bottom],
        [0, 0, left, height],
        [right, 0, width - right, height],
      ]) {
        if (strip[2] > 0 && strip[3] > 0) {
          chrome.push(strip);
        }
      }
    }
    for (const element of document.querySelectorAll('.sd-player > :not(.sd-frame)')) {
      const box = element.getBoundingClientRect();
      if (box.width > 0 && box.height > 0) {
        chrome.push([box.x, box.y, box.width, box.height]);
      }
    }
    return {
      mounted: watch.mounted,
      body: watch.body,
      errors: watch.errors.slice(),
      position: watch.player === null ? null : watch.player.position,
      chrome,
      aligned,
    };
  };
})();
