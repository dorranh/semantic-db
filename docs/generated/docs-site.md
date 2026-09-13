# Maintaining the documentation site

From the repository root, run:

```sh
just docs
```

Prerequisites are `just`, Node.js 22.12 or newer (Node 24 is used in CI), and npm.
The recipe runs `npm ci` from the committed lockfile, then starts Astro on
[localhost:4321/semantic-db/](http://localhost:4321/semantic-db/). The first install
requires access to the npm registry. Stop an interactive server with Ctrl-C.
Astro can automatically background the server in agent environments; use
`npm --prefix docs/site run stop` to stop that server. If the default
port is occupied, use the URL printed by Astro. To choose a port:

```sh
just docs --port 4400
```

## Content ownership

Existing Markdown under `docs/` stays the curated source of truth. New generated
guides belong under `docs/generated/`. Do not rewrite top-level documentation
as part of site maintenance without asking the maintainer.

The Astro content loader reads `docs/*.md` and `docs/generated/**/*.md` directly;
no copied Markdown or required frontmatter. Titles come from each document's
first H1. Changes appear during development. The generated `site-home.md` is the
landing page. Site code and configuration live under `docs/site/`.

Navigation is explicitly ordered in `src/lib/navigation.ts`. Other loaded pages
remain available under “Engineering notes & history,” which displays a notice
that older assessments may describe earlier behavior. Adding a Markdown file
does not automatically promote it to the primary guides.

Document paths map to lowercase URLs without `.md`, including
`generated/README.md` at `/semantic-db/generated/readme/`. Keep a single H1 and
use H2 headings for the automatic page outline. Relative Markdown links become
site links at build time; links to source files, examples, or JSON artifacts point
to GitHub. Links to the three moved assessments are resolved to `generated/spikes`
without editing the curated documents. External links are preserved.

## Build and verify

```sh
just docs-build
```

This installs locked dependencies, builds the static site, and validates internal
page links, assets, heading anchors, and repository source paths. Output is
`docs/site/dist/`. External
links are not network-checked by the build. To inspect production output:

```sh
npm --prefix docs/site run preview
```

The site has responsive navigation, heading links, syntax highlighting, and
horizontal scrolling for wide tables/code. Fonts have local system fallbacks;
Google Fonts enhances typography when reachable. Full-text search is not included
in this bootstrap.

## GitHub Pages

The [workflow](../../.github/workflows/docs.yml) builds changed docs on pull
requests and deploys successful builds on `main` or a manual run on `main`.
In the repository's **Settings → Pages**, select **GitHub Actions** as the source.
That one-time repository setting is required; adding the workflow does not enable
Pages by itself.

The configured published address is
[dorranh.github.io/semantic-db/](https://dorranh.github.io/semantic-db/).
The [Astro GitHub Pages guide](https://docs.astro.build/en/guides/deploy/github/)
explains the `site` and `base` settings. If moving the site, update both
`astro.config.mjs` and the shared link constants in
`scripts/documentation-links.mjs`, plus the workflow as appropriate. Local
development deliberately uses the same base path so broken deployment links can
be caught before publishing.
