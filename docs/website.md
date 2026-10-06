# Glance download site

The download site at https://glancepc.com, in `website/`: a static page in
Chinese (`index.html`) and English (`en/index.html`), one stylesheet and two
scripts in `assets/` (`site.js`: downloads, release notes, the phone note;
`demo.js`: the panel drawn in the page on made-up readings, which slides out
of the window's right edge and shows the three looks). No build step.

The pages load `site.css`, `site.js` and `demo.js` with `?v=<n>`: after
changing any of them, raise `n` in `index.html`, `en/index.html` and
`404.html`, or browsers keep the old file for up to an hour next to the new
page.

## Deploy on Cloudflare Pages

It is the Pages project `glancepc` (custom domains glancepc.com and
www.glancepc.com), deployed by direct upload:

    npx wrangler@4 pages deploy website --project-name glancepc --branch main

with `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID` set. Or, through the
dashboard:

1. Cloudflare dashboard → Workers & Pages → Create → Pages → connect the
   `lulu-loopp/glance` repository (or upload this folder directly).
2. Build settings: framework preset **None**, build command **empty**, build
   output directory **`website`** (with a Git connection, set the root
   directory to `website` and the output directory to `/`, or leave the root
   empty and the output as `website`).
3. Production branch `main`. Every push that touches `website/` redeploys it.
4. Custom domains → add the domain. Every link in the site is relative, so
   nothing needs changing for it (`404.html` uses root paths, which suit any
   domain).

`_headers` sets the security headers (a CSP that allows only this site, the
GitHub API and the Gitee API) and caching: pages are revalidated on every
visit, `site.css` and `site.js` are cached for an hour, images and the font
for a month or more and marked immutable.

## What updates itself

At page load the script asks both the GitHub and the Gitee API for the latest
releases (both allow cross-origin requests). From the answers it:

- points the download buttons at the installer itself
  (`Glance_<version>_x64-setup.exe`) instead of the release pages;
- shows the version and the installer's size;
- renders the last four releases' notes in the page's language (each
  release's notes are English and Chinese halves split by a `---` line).

Visitors whose browser language starts with `zh` get Gitee as the main
button and GitHub as the second link; everyone else the other way round.
Phones are told it's a Windows app and offered to copy the address.

## After a release

Nothing is required: the page picks up the new version by itself. The text
written into the HTML is only what shows without JavaScript or when both
APIs fail, so refresh it now and then:

- In both `index.html` and `en/index.html`, update the two download buttons'
  `data-version`, `data-size` and visible text (`0.1.7 版 · 免费 · 4.1 MB` /
  `Version 0.1.7 · free · 4.1 MB`).
- Replace the release entries inside `data-changelog` (the first is an
  `<article>` marked latest; the rest are `<details>`), copying the notes'
  Chinese half into `index.html` and the English half into `en/index.html`.

## Images

`assets/img` holds downscaled WebP copies of `docs/images`: `hero-*` is the
right-hand part of `banner-*`, `desktop-*` and `looks-*` come in two widths
for `srcset`. Because they are cached as immutable, give a changed image a
new file name (and update the references) rather than overwriting it.

`assets/fonts/archivo-600.woff2` is Archivo SemiBold, cut to printable ASCII
(6 KB), used for the wordmark, headings and numbers; Chinese text uses the
system's fonts (PingFang SC, Microsoft YaHei). Its licence is
`assets/fonts/Archivo-OFL.txt`.
