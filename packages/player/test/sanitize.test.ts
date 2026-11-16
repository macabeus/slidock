// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest';

import { mediaPath, resolveMedia } from '../src/deck';
import { isSafeCss, isSafeLink, isSafeResource, sanitize } from '../src/sanitize';

function fragment(html: string): DocumentFragment {
  const template = document.createElement('template');
  template.innerHTML = html;
  return template.content;
}

function html(root: DocumentFragment): string {
  const div = document.createElement('div');
  div.append(root.cloneNode(true));
  return div.innerHTML;
}

describe('sanitizer', () => {
  it('keeps the format vocabulary as printed', () => {
    const markup =
      '<main data-slide="s"><div data-el="s.a" style="position:absolute;left:1pt"><p data-p="0">Hi <b>there</b></p>' +
      '</div><svg viewBox="0 0 10 10"><defs><linearGradient id="g"><stop offset="0"></stop></linearGradient>' +
      '<clipPath id="c"><rect width="1" height="1"></rect></clipPath></defs>' +
      '<path d="M0 0L1 1" style="fill:url(#g)"></path></svg>' +
      '<video src="media/x.mp4" poster="media/x.png" preload="none" data-autoplay=""></video>' +
      '<a href="https://example.com">l</a><a href="#slide=s2">j</a></main>';
    const root = fragment(markup);
    const report = sanitize(root);
    expect(report).toEqual({ elements: [], attributes: [] });
    expect(html(root)).toBe(markup);
  });

  it('drops elements outside the allow-list with their subtrees', () => {
    const root = fragment(
      '<main><script>alert(1)</script><iframe srcdoc="x"></iframe><div><style>p{}</style><form><p>x</p></form>' +
        '<svg><foreignObject><p>y</p></foreignObject><animate></animate></svg><object></object></div></main>',
    );
    const report = sanitize(root);
    expect(report.elements.sort()).toEqual(['animate', 'foreignObject', 'form', 'iframe', 'object', 'script', 'style']);
    expect(html(root)).toBe('<main><div><svg></svg></div></main>');
  });

  it('drops event handlers and script URLs', () => {
    const root = fragment(
      '<main onclick="x()"><img src="media/a.png" onerror="x()" alt=""><a href="javascript:x()">a</a>' +
        '<a href="java\tscript:x()">b</a><svg><use href="data:image/svg+xml,x" onload="x()"></use></svg></main>',
    );
    sanitize(root);
    expect(html(root)).toBe(
      '<main><img src="media/a.png" alt=""><a>a</a><a>b</a><svg><use href="data:image/svg+xml,x"></use></svg></main>',
    );
  });

  it('reads a URL scheme the way browsers do, past tabs and newlines', () => {
    const root = fragment('<img src="java&#9;script:x()" alt=""><video poster=" &#10;javascript:x()"></video>');
    expect(sanitize(root).attributes).toEqual(['img@src', 'video@poster']);
  });

  it('keeps links to #, http, https and mailto only', () => {
    expect(['#x', 'http://a', 'HTTPS://a', 'mailto:a@b', ' https://a '].every(isSafeLink)).toBe(true);
    expect(['file:///etc', 'data:text/html,x', 'ftp://a', 'media/a.png', ''].some(isSafeLink)).toBe(false);
    const root = fragment('<a href="file:///etc/passwd">x</a>');
    expect(sanitize(root).attributes).toEqual(['a@href']);
  });

  it('lets resources reach the deck media only', () => {
    expect(
      ['media/a.png', '../media/a.png', '#g', 'data:image/png;base64,AA', 'data:video/mp4;base64,AA'].every(
        isSafeResource,
      ),
    ).toBe(true);
    expect(
      ['https://evil/x.png', '//evil/x.png', 'blob:x', 'file:///x', 'data:text/html,x', 'javascript:x'].some(
        isSafeResource,
      ),
    ).toBe(false);
    const root = fragment(
      '<img src="https://evil/t.png"><div style="background-image:url(&quot;https://evil/x&quot;)"></div>' +
        '<div style="background-image:url(&quot;media/ok.png&quot;)"></div><img srcset="media/a.png 1x">',
    );
    const report = sanitize(root);
    expect(report.attributes).toEqual(['img@src', 'div@style', 'img@srcset']);
    expect(html(root)).toContain('url(&quot;media/ok.png&quot;)');
  });

  it('reads a resource URL the way browsers do: only paths inside the deck folder are its own', () => {
    // Backslashes and tabs make these network-path references on http(s) and file: pages.
    const foreign = [
      '\\\\evil/x.png',
      '/\\evil/x.png',
      '\\/evil/x.png',
      '/\t/evil/x.png',
      ' //evil/x.png',
      'https:evil/x.png',
      'http:evil/x.png',
      'file:evil/x.png',
      'C:/x.png',
      '/media/a.png',
      '../../a.png',
      '%2e%2e/%2e%2e/a.png',
      'data: text/html,x',
    ];
    for (const value of foreign) {
      expect(isSafeResource(value), value).toBe(false);
    }
    const own = ['media/a.png', './media/a.png', '../media/a.png', 'media/a%20b.png', '#g', '', 'DATA:image/png,AA'];
    for (const value of own) {
      expect(isSafeResource(value), value).toBe(true);
    }
    const root = fragment('<img src="\\\\evil/x.png" alt=""><video poster="/&#9;/evil/p.png"></video>');
    expect(sanitize(root).attributes).toEqual(['img@src', 'video@poster']);
  });

  it('reads CSS the way browsers do: every load is a deck resource', () => {
    const refused = [
      "background-image:image-set('https://evil/x.png' 1x)",
      'background-image:-webkit-image-set("media/a.png" 1x)',
      'background-image:Cross-Fade(url(media/a.png), url(media/b.png), 50%)',
      // A custom property can hand a string to a loader that the value names only through var().
      '--x:image-set("https://evil/x.png" 1x)',
      'background-image:\\75 rl(https://evil/x.png)',
      'background-image:u\\rl(https://evil/x.png)',
      'background-image:url(\\\\evil/x.png)',
      'background-image:url("\\5c\\5c evil/x.png")',
      'background-image:URL( https://evil/x.png )',
      'background-image:url(media/a b.png)',
      'font-family:"/*";background-image:url(https://evil/x.png);content:"*/"',
      'color:red/* unclosed',
      'font-family:"unclosed',
      'background:var(--a, "x")',
    ];
    for (const value of refused) {
      expect(isSafeCss(value), value).toBe(false);
    }
    const kept = [
      'position:absolute;left:1pt;background-color:var(--sd-color-lt1,#ffffff)',
      'clip-path:path("M0 0L1 1Z")',
      'fill:url(#g)',
      'background-image:url( "media/a b.png" )',
      "background-image:url('../media/a.png')",
      'font-family:"Fo\\22 o\\5c ", sans-serif',
      'color:red/* url(https://evil/x.png) */',
      'content:"https://shown/as/text"',
      'clip:rect(0 0 0 0)',
    ];
    for (const value of kept) {
      expect(isSafeCss(value), value).toBe(true);
    }
  });

  it('checks SVG presentation attributes as CSS', () => {
    const root = fragment(
      '<svg><rect fill="url(https://evil/f.svg#p)" filter="url(\\\\evil/f.svg#x)" mask="url(#m)" ' +
        'clip-path="url(#c)" stroke-width="2" transform="translate(1 2)"></rect>' +
        '<path d="M0 0L1 1" marker-end="url(//evil/m.svg#e)"></path></svg>',
    );
    expect(sanitize(root).attributes).toEqual(['rect@fill', 'rect@filter', 'path@marker-end']);
    expect(html(root)).toBe(
      '<svg><rect mask="url(#m)" clip-path="url(#c)" stroke-width="2" transform="translate(1 2)"></rect>' +
        '<path d="M0 0L1 1"></path></svg>',
    );
  });

  it('drops comments', () => {
    const root = fragment('<main><!-- x --><p>a</p></main>');
    sanitize(root);
    expect(html(root)).toBe('<main><p>a</p></main>');
  });
});

describe('media resolution', () => {
  it('reads the deck media path of a markup URL', () => {
    expect(mediaPath('media/ab.png')).toBe('media/ab.png');
    expect(mediaPath('../media/ab.png')).toBe('media/ab.png');
    expect(mediaPath('https://x/media/ab.png')).toBeNull();
    expect(mediaPath('#g')).toBeNull();
  });

  it('points attributes and style URLs at the resolver', () => {
    const root = fragment(
      '<img src="media/a.png"><video src="media/v.mp4" poster="media/p.png"></video>' +
        '<svg><image href="media/i.png"></image></svg>' +
        '<div style="left:0;background-image:url(&quot;media/b.png&quot;)"></div><a href="#x">x</a>',
    );
    resolveMedia(root, (path) => `blob:app/${path.slice(6)}`);
    const out = html(root);
    expect(out).toContain('src="blob:app/a.png"');
    expect(out).toContain('src="blob:app/v.mp4"');
    expect(out).toContain('poster="blob:app/p.png"');
    expect(out).toContain('href="blob:app/i.png"');
    expect(out).toContain('url(&quot;blob:app/b.png&quot;)');
    expect(out).toContain('href="#x"');
  });
});
