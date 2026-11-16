// The motion and shell lanes' page side, evaluated before the deck page's own scripts. It takes the deck page over:
// the page's `Slidock.mount` does nothing, and `boot` mounts the deck's own player presenting, on a host of the deck's
// pixel size, with keys, pointer, chrome and auto-advance timers off, so only the lane moves it. Every Web Animation
// the player creates is recorded and paused at its start: the lane seeks the batch to any time, screenshots, then
// finishes it, and the player goes on as it does when its animations end. An endless repeat loops at the times the
// lane seeks until the lane cuts it, as a click does: from then on it holds where its round ends. Media never play,
// and a video shows its poster as an image in its box (the static lane's render policy), so a frame never depends on
// decoding.
(() => {
  const realAnimate = Element.prototype.animate;
  /** Every animation created, in creation order. */
  const records = [];
  /** The properties the player's animations animate, per element. */
  const animatedKeys = new WeakMap();
  /** The properties an animation's keyframes name. */
  const keysOfEffect = (effect) => {
    const keys = new Set();
    for (const keyframe of effect && effect.getKeyframes ? effect.getKeyframes() : []) {
      for (const key of Object.keys(keyframe)) {
        if (key !== 'offset' && key !== 'computedOffset' && key !== 'easing' && key !== 'composite') {
          keys.add(key);
        }
      }
    }
    return keys;
  };
  /** Whether new animations are the player's moves (a still render's own holds are not). */
  let capturing = true;
  Element.prototype.animate = function animate(keyframes, options) {
    const animation = realAnimate.call(this, keyframes, options);
    if (capturing) {
      try {
        animation.pause();
      } catch {
        // An animation that cannot pause runs; the lane finishes it with its batch.
      }
    }
    records.push({ animation, target: this, options, driven: capturing, reported: false });
    if (capturing) {
      const keys = animatedKeys.get(this) ?? new Set();
      for (const key of keysOfEffect(animation.effect)) {
        keys.add(key);
      }
      animatedKeys.set(this, keys);
    }
    return animation;
  };
  HTMLMediaElement.prototype.play = function play() {
    return Promise.resolve();
  };

  let slidock;
  Object.defineProperty(window, 'Slidock', {
    configurable: true,
    get: () => slidock,
    set(value) {
      slidock =
        value !== null && typeof value === 'object' ? Object.create(value, { mount: { value: () => null } }) : value;
    },
  });

  const macrotask = () => new Promise((resolve) => setTimeout(resolve, 0));
  const frame = () => new Promise((resolve) => requestAnimationFrame(() => resolve()));

  let player = null;
  let host = null;
  let stillHost = null;
  let disposeStill = null;
  /** The slide a move is bringing on screen, whose readiness gate the next batch waits for. */
  let moving = null;
  const diagnostics = [];
  const ready = [];
  const nodes = new WeakMap();
  /** Each numbered node, by its number. */
  const byId = new Map();
  let nodeCount = 0;
  const nodeId = (element) => {
    if (!nodes.has(element)) {
      nodes.set(element, nodeCount);
      byId.set(nodeCount, element);
      nodeCount++;
    }
    return nodes.get(element);
  };

  const finiteEnd = (animation) => {
    const effect = animation.effect;
    if (!effect) {
      return 0;
    }
    const end = effect.getComputedTiming().endTime;
    return Number.isFinite(end) ? end : null;
  };
  /** The player's animations it still waits for: not finished, not cancelled. */
  const pending = () =>
    records.filter(
      (record) => record.driven && record.animation.playState !== 'finished' && record.animation.playState !== 'idle',
    );

  const layerOf = (element) => {
    if (element.classList && element.classList.contains('sd-layer')) {
      return element;
    }
    const root = element.getRootNode();
    const shadowHost = root instanceof ShadowRoot ? root.host : null;
    return shadowHost !== null && shadowHost.classList.contains('sd-layer') ? shadowHost : null;
  };
  const roleOf = (layer) => {
    if (layer === null) {
      return 'stage';
    }
    if (layer.hasAttribute('data-sd-clone')) {
      return `clone-${layer.getAttribute('data-sd-clone')}`;
    }
    if (layer.hasAttribute('data-sd-incoming')) {
      return 'incoming';
    }
    return layer.hasAttribute('data-sd-current') ? 'current' : 'other';
  };
  const text = (value) => (value === undefined || value === null ? null : String(value));
  const describe = (record) => {
    const { animation, target } = record;
    const effect = animation.effect;
    const timing = effect ? effect.getTiming() : {};
    const keys = keysOfEffect(effect);
    const layer = layerOf(target);
    const owner = target.closest ? target.closest('[data-el]') : null;
    const paragraph = target.closest ? target.closest('[data-p]') : null;
    return {
      node: nodeId(target),
      element: owner === null ? null : owner.getAttribute('data-el'),
      paragraph: paragraph === null ? null : Number(paragraph.getAttribute('data-p')),
      split: target.hasAttribute ? target.hasAttribute('data-sd-unit') : false,
      layer: layer === null ? null : layer.getAttribute('data-sd-layer'),
      role: roleOf(layer),
      whole: layer === target,
      keys: [...keys].sort(),
      delay: Number(timing.delay ?? 0),
      duration: typeof timing.duration === 'number' ? timing.duration : 0,
      iterations: Number.isFinite(timing.iterations) ? timing.iterations : -1,
      direction: text(timing.direction),
      easing: text(timing.easing),
      fill: text(timing.fill),
      end: finiteEnd(animation),
    };
  };

  /** The element above `node`, crossing out of a shadow root to its host. */
  const parentOf = (node) => {
    if (node.parentElement !== null) {
      return node.parentElement;
    }
    const root = node.getRootNode();
    return root instanceof ShadowRoot ? root.host : null;
  };
  /** The properties the player's animations on `element` animate. */
  const keysOf = (element) => [...(animatedKeys.get(element) ?? [])].sort();
  const round = (value) => Math.round(value * 100) / 100;
  /**
   * What a node shows now: its border box on the page with transforms applied, the computed value of each property its
   * animations animate, and whether it shows at all (connected, visible, and not transparent up to the page).
   */
  const probeNode = (id) => {
    const element = byId.get(id);
    if (element === undefined) {
      return null;
    }
    const pose = {};
    const connected = element.isConnected;
    if (!connected) {
      return { node: id, rect: [0, 0, 0, 0], pose, shown: false, connected };
    }
    const style = getComputedStyle(element);
    const keys = keysOf(element);
    for (const key of keys) {
      pose[key] = key.startsWith('--') ? style.getPropertyValue(key).trim() : String(style[key] ?? '');
    }
    // A registered custom property shows only through the mask image that reads it.
    if (keys.some((key) => key.startsWith('--'))) {
      pose.maskImage = String(style.maskImage || style.webkitMaskImage || 'none');
    }
    const box = element.getBoundingClientRect();
    let opacity = 1;
    for (let at = element; at !== null; at = parentOf(at)) {
      opacity *= Number(getComputedStyle(at).opacity);
    }
    return {
      node: id,
      rect: [round(box.x), round(box.y), round(box.width), round(box.height)],
      pose,
      shown: style.visibility === 'visible' && opacity > 0.001,
      connected,
    };
  };

  /** Waits until the player stops creating animations: two quiet macrotasks and a frame. */
  async function quiet() {
    let seen = -1;
    for (let round = 0; round < 1000 && seen !== records.length; round++) {
      seen = records.length;
      await macrotask();
      await macrotask();
      await frame();
    }
  }

  /** The next batch of the move, or `null` once the move is over (its slide shown, nothing left to play). */
  async function batch() {
    for (let round = 0; round < 10000; round++) {
      await quiet();
      const live = pending();
      if (live.length > 0) {
        const fresh = records.filter((record) => !record.reported);
        for (const record of fresh) {
          record.reported = true;
        }
        const ends = live.map((record) => finiteEnd(record.animation)).filter((end) => end !== null);
        // A still state's holds finish as they are made; the batch is what waits to play.
        return {
          animations: fresh.filter((record) => live.includes(record)).map(describe),
          durationMs: Math.max(0, ...ends),
          // The player marks the slide coming in while its transition runs.
          transition: host.querySelector('.sd-layer[data-sd-incoming]') !== null,
        };
      }
      if (moving === null || ready.includes(moving)) {
        moving = null;
        return null;
      }
      // The readiness gate: the slide's fonts and images.
      await new Promise((resolve) => {
        const off = player.on('ready', () => {
          off();
          resolve();
        });
      });
    }
    throw new Error('the move never settled');
  }

  const report = (next) => ({ batch: next, position: player.position, diagnostics: diagnostics.splice(0) });

  /**
   * Cuts an endless repeat as a click does: it ends with its round (two iterations when it plays back and forth) and
   * holds there. As the player holds a cut repeat, the hold is a still frame of that end made after a style read with
   * nothing animating (a layer rasterised while the repeat ran would paint differently), and an end that shows what
   * the element shows without it holds nothing; the frame takes the repeat's place in the same animation, which the
   * player cancels when it cuts the repeat itself.
   */
  const cutLoop = (record) => {
    const { animation } = record;
    const effect = animation.effect;
    if (!effect || finiteEnd(animation) !== null) {
      return;
    }
    const direction = effect.getTiming().direction;
    effect.updateTiming({ iterations: direction === 'alternate' || direction === 'alternate-reverse' ? 2 : 1 });
    animation.finish();
    const target = effect.target;
    if (!target) {
      return;
    }
    const keys = [...keysOfEffect(effect)];
    const read = () => {
      const style = getComputedStyle(target);
      return Object.fromEntries(
        keys.map((key) => [key, key.startsWith('--') ? style.getPropertyValue(key).trim() : String(style[key])]),
      );
    };
    const frame = read();
    animation.cancel();
    const rest = read();
    if (keys.some((key) => frame[key] !== rest[key])) {
      animation.effect = new KeyframeEffect(target, [frame], { duration: 0, fill: 'forwards' });
      animation.finish();
    }
  };

  window.__sdMotion = {
    /** Mounts the deck's own player on a host of the deck's pixel size; it starts at the first shown slide. */
    boot() {
      if (typeof slidock !== 'object' || slidock === null) {
        throw new Error('the deck page has no player');
      }
      // A deck opened from its own files gets each animated GIF's first frame from the lane (the router serves it so).
      const stills = window.__sdMotionStills ?? {};
      for (const template of document.querySelectorAll('template[data-sd-slide]')) {
        for (const image of template.content.querySelectorAll('img[src]')) {
          const still = stills[image.getAttribute('src')];
          if (typeof still === 'string') {
            image.setAttribute('src', still);
          }
        }
        for (const video of template.content.querySelectorAll('video[poster]')) {
          const image = document.createElement('img');
          for (const name of ['data-el', 'class']) {
            if (video.hasAttribute(name)) {
              image.setAttribute(name, video.getAttribute(name));
            }
          }
          image.setAttribute('src', video.getAttribute('poster'));
          image.setAttribute('alt', '');
          image.setAttribute('style', `${video.getAttribute('style') ?? ''};display:block`);
          video.replaceWith(image);
        }
      }
      const source = slidock.inlineDeck(document);
      for (const slide of source.deck.slides) {
        if (slide.transition !== null && typeof slide.transition === 'object') {
          slide.transition.advance = null;
        }
      }
      const width = (source.deck.size.w * 4) / 3;
      const height = (source.deck.size.h * 4) / 3;
      document.body.replaceChildren();
      document.body.style.margin = '0';
      host = document.createElement('div');
      host.style.cssText = `position:fixed;left:0;top:0;width:${width}px;height:${height}px;overflow:hidden`;
      document.body.append(host);
      player = slidock.createPlayer(host, source, {
        mode: 'present',
        keyboard: 'off',
        pointer: 'off',
        chrome: false,
        reducedMotion: 'never',
        readyTimeoutMs: 60000,
      });
      player.on('diagnostic', (event) => diagnostics.push(event));
      player.on('ready', (event) => ready.push(event.slide));
      moving = player.position.slide;
      return { position: player.position, version: slidock.version, visibility: document.visibilityState };
    },
    /** The first batch of the move under way (the first slide's, after `boot`). */
    async pending() {
      return report(await batch());
    },
    /** Cuts to a slide at a step, plays nothing, and waits until it shows. */
    async cut(slide, step) {
      ready.length = 0;
      const before = player.position;
      player.goTo(slide, step);
      const after = player.position;
      moving = after.slide !== before.slide || after.step !== before.step ? after.slide : null;
      for (let next = await batch(); next !== null; next = await batch()) {
        this.end();
      }
      return report(null);
    },
    /** Moves on; resolves with the move's first batch, or `null` when it plays nothing. */
    async next() {
      ready.length = 0;
      const before = player.position.slide;
      player.next();
      moving = player.position.slide === before ? null : player.position.slide;
      return report(await batch());
    },
    /**
     * Seeks the batch to `ms` (paused), an endless repeat looping; with `cut`, cuts the endless repeats first. Resolves
     * with what each animated node shows then.
     */
    async seek(ms, cut) {
      const live = pending();
      for (const record of live) {
        if (cut) {
          cutLoop(record);
        }
        record.animation.currentTime = ms;
      }
      await frame();
      await frame();
      const ids = [...new Set(live.map((record) => nodeId(record.target)))];
      return { nodes: ids.map(probeNode).filter((probe) => probe !== null) };
    },
    /** What each numbered node shows now (see `seek`). */
    probe(ids) {
      return ids.map(probeNode).filter((probe) => probe !== null);
    },
    /** Waits `n` frames. */
    async frames(n) {
      for (let i = 0; i < n; i++) {
        await frame();
      }
      return null;
    },
    /** Finishes the batch, cutting its endless repeats. */
    end() {
      for (const record of pending()) {
        cutLoop(record);
        record.animation.finish();
      }
    },
    /** Finishes the batch and resolves with the move's next batch, or `null` once the move is over. */
    async finish() {
      this.end();
      return report(await batch());
    },
    /** Whether each unit (`[element, paragraph | null]`) of the shown slide shows: visible, and not transparent. */
    visible(units) {
      const layer = host.querySelector('.sd-layer[data-sd-current]');
      const root = layer === null ? null : layer.shadowRoot;
      const elements = new Map();
      for (const element of root === null ? [] : root.querySelectorAll('[data-el]')) {
        const id = element.getAttribute('data-el');
        if (!elements.has(id)) {
          elements.set(id, element);
        }
      }
      return units.map(([element, paragraph]) => {
        let node = elements.get(element);
        if (node !== undefined && paragraph !== null) {
          node = [...node.querySelectorAll('[data-p]')].find((p) => p.getAttribute('data-p') === String(paragraph));
        }
        if (node === undefined) {
          return null;
        }
        if (getComputedStyle(node).visibility !== 'visible') {
          return false;
        }
        let opacity = 1;
        for (let at = node; at !== null; at = at.parentElement) {
          opacity *= Number(getComputedStyle(at).opacity);
        }
        return opacity > 0.001;
      });
    },
    /**
     * The words the shown slide draws, as the static lane measures them (`measure.js`): each with its box in slide
     * points (`widthPt` is the slide's width), its element's IR id, its text box, its font size in slide points, and
     * whether it starts and ends its text node (the lane joins a word split across nodes as `measure.js` does); a word
     * counts only where it can be seen.
     */
    words(widthPt) {
      const layer = host.querySelector('.sd-layer[data-sd-current]');
      const main = layer === null || layer.shadowRoot === null ? null : layer.shadowRoot.querySelector('main');
      if (main === null) {
        return [];
      }
      const slide = main.getBoundingClientRect();
      const scale = slide.width > 0 ? widthPt / slide.width : 0;
      const unit = main.offsetWidth > 0 ? widthPt / main.offsetWidth : 0;
      const textBoxes = new Map([...main.querySelectorAll('[data-sd-text]')].map((box, index) => [box, index]));
      const alphaOf = (color) => {
        const match = /rgba?\(([^)]+)\)/.exec(color);
        const parts = match === null ? [] : match[1].split(/[\s,/]+/).filter(Boolean);
        return parts.length >= 4 ? Number(parts[3]) : 1;
      };
      const words = [];
      const walker = document.createTreeWalker(main, NodeFilter.SHOW_TEXT);
      for (let node = walker.nextNode(); node !== null; node = walker.nextNode()) {
        const parent = node.parentElement;
        const text = node.nodeValue ?? '';
        if (parent === null || !/\S/.test(text)) {
          continue;
        }
        let opacity = alphaOf(getComputedStyle(parent).color);
        let shown = true;
        for (let at = parent; at !== null && at !== main.parentNode; at = at.parentElement) {
          const style = getComputedStyle(at);
          if (style.display === 'none' || style.visibility === 'hidden') {
            shown = false;
            break;
          }
          opacity *= Number(style.opacity);
        }
        if (!shown || opacity <= 0.05) {
          continue;
        }
        for (const match of text.matchAll(/\S+/g)) {
          const range = document.createRange();
          range.setStart(node, match.index ?? 0);
          range.setEnd(node, (match.index ?? 0) + match[0].length);
          const rects = [...range.getClientRects()].filter((r) => r.width > 0 && r.height > 0);
          if (rects.length === 0) {
            continue;
          }
          const left = Math.max(slide.left, Math.min(...rects.map((r) => r.left)));
          const top = Math.max(slide.top, Math.min(...rects.map((r) => r.top)));
          const right = Math.min(slide.right, Math.max(...rects.map((r) => r.right)));
          const bottom = Math.min(slide.bottom, Math.max(...rects.map((r) => r.bottom)));
          if (right <= left || bottom <= top) {
            continue;
          }
          const start = match.index ?? 0;
          const textBox = parent.closest('[data-sd-text]');
          words.push({
            text: match[0],
            x: (left - slide.left) * scale,
            y: (top - slide.top) * scale,
            w: (right - left) * scale,
            h: (bottom - top) * scale,
            joinBefore: start === 0,
            joinAfter: start + match[0].length === text.length,
            el: parent.closest('[data-el]')?.getAttribute('data-el') ?? null,
            textBox: textBox === null ? null : (textBoxes.get(textBox) ?? null),
            size: Number.parseFloat(getComputedStyle(parent).fontSize) * unit,
          });
        }
      }
      return words;
    },
    /** Shows the player's still render of a slide at a step over the stage; `still(null)` takes it away. */
    async still(slide, step) {
      if (disposeStill !== null) {
        disposeStill();
        disposeStill = null;
      }
      if (stillHost !== null) {
        stillHost.remove();
        stillHost = null;
      }
      if (slide === null) {
        await frame();
        return null;
      }
      capturing = false;
      try {
        stillHost = document.createElement('div');
        stillHost.style.cssText = `${host.style.cssText};z-index:10;background:#fff`;
        document.body.append(stillHost);
        disposeStill = player.renderStatic(slide, step, stillHost);
      } finally {
        capturing = true;
      }
      const layer = stillHost.querySelector('.sd-layer');
      const root = layer === null ? null : layer.shadowRoot;
      await document.fonts.ready;
      if (root !== null) {
        await Promise.all(Array.from(root.querySelectorAll('img'), (image) => image.decode().catch(() => undefined)));
      }
      await frame();
      await frame();
      return null;
    },
  };
})();
