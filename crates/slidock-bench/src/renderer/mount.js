async function mountSlide(slideId, width, height, endState) {
  // The deck page has booted its player; the host lane renders one slide as the player's still render does, in a
  // shadow root, at the deck's own size, under the same render policy as the standalone document. The still render
  // fits the slide to the target's width: `contain` fits to the target's integer client size, which scales a slide of
  // fractional pixel height by a hair. The still render shows the slide after its last build, where an exit build has
  // hidden its element; the standalone document shows every element. Unless `endState`, the build state is cleared,
  // so the two show the same elements.
  if (typeof Slidock !== 'object' || Slidock === null) {
    throw new Error('the deck page has no player');
  }
  const source = Slidock.inlineDeck(document);
  const index = source.deck.slides.findIndex((slide) => slide.id === slideId);
  if (index < 0) {
    throw new Error(`the deck has no slide ${slideId}`);
  }
  document.body.replaceChildren();
  const target = document.createElement('div');
  target.style.cssText = `position:fixed;left:0;top:0;width:${width}px;height:${height}px;overflow:hidden`;
  document.body.append(target);
  Slidock.renderStatic(source, index, Number.MAX_SAFE_INTEGER, target, 'width');
  const layer = target.querySelector('[data-sd-layer]');
  const root = layer === null ? null : layer.shadowRoot;
  if (root === null) {
    throw new Error(`slide ${slideId} mounted no shadow root`);
  }
  if (!endState) {
    for (const element of Array.from(root.querySelectorAll('[data-sd-state]'))) {
      element.removeAttribute('data-sd-state');
    }
  }
  for (const video of Array.from(root.querySelectorAll('video[poster]'))) {
    const image = document.createElement('img');
    image.setAttribute('src', video.getAttribute('poster'));
    image.setAttribute('alt', '');
    image.setAttribute('style', `${video.getAttribute('style') ?? ''};display:block`);
    video.replaceWith(image);
  }
  await document.fonts.ready;
  await Promise.all(Array.from(root.querySelectorAll('img'), (image) => image.decode().catch(() => undefined)));
  return { slides: source.deck.slides.length, index };
}
