import { defineCollection } from 'astro:content';
import { glob } from 'astro/loaders';
import { docsSchema } from '@astrojs/starlight/schema';

const published = [
  'index.md', 'comparison.md', 'appendices.md', 'DOCUMENTATION_PLAN.md', 'format-spec.md',
  'export-context.md', 'plugin-api.md', 'mcp.md', 'psd-import.md',
  'graph-editor.md', 'rigging-walkthrough.md', 'getting-started/**/*.md',
  'animator/**/*.md', 'formats/**/*.md', 'automation/**/*.md',
  'reference/**/*.md', 'developer/index.md',
];

export const collections = {
  docs: defineCollection({
    loader: glob({ base: './docs', pattern: published }),
    schema: docsSchema(),
  }),
};
