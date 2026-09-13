import { defineConfig } from 'astro/config';
import { unified } from '@astrojs/markdown-remark';
import { documentationLinks } from './scripts/documentation-links.mjs';

export default defineConfig({
  site: 'https://dorranh.github.io',
  base: '/semantic-db',
  trailingSlash: 'always',
  output: 'static',
  markdown: {
    processor: unified({ remarkPlugins: [documentationLinks] }),
    shikiConfig: { theme: 'github-light' },
  },
});
