import { defineCollection } from 'astro:content';
import { glob } from 'astro/loaders';

export const collections = {
  docs: defineCollection({
    loader: glob({
      base: '..',
      pattern: ['*.md', 'generated/**/*.md'],
      generateId: ({ entry }) => entry.replace(/\.md$/i, '').toLowerCase(),
    }),
  }),
};
