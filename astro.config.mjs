import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

export default defineConfig({
  site: 'https://ankhimate.github.io',
  base: '/editor',
  integrations: [
    starlight({
      title: 'Ankhimate',
      description: 'Rig, animate, and export 2D characters.',
      favicon: '/favicon.svg',
      customCss: ['./src/styles/ankhimate.css'],
      social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/Ankhimate/editor' }],
      editLink: { baseUrl: 'https://github.com/Ankhimate/editor/edit/main/docs/' },
      sidebar: [
        { label: 'Start here', items: ['index', 'getting-started/status', 'getting-started/install'] },
        {
          label: 'Animator manual',
          items: [
            'animator', 'animator/workspace', 'animator/rigging',
            'animator/deformation', 'animator/constraints', 'animator/animation', 'animator/import-export',
            'animator/recipes', 'animator/troubleshooting',
            { label: 'Rigging walkthrough', slug: 'rigging-walkthrough' },
            { label: 'PSD import', slug: 'psd-import' },
            { label: 'Graph editor', slug: 'graph-editor' },
          ],
        },
        { label: 'Formats and export', items: ['formats', 'formats/ankh-v3', 'formats/migrations', 'formats/export-runtime', 'export-context'] },
        { label: 'Plugins and automation', items: ['automation', 'automation/plugins', 'plugin-api', 'reference/document-verbs', 'automation/mcp', 'mcp', 'reference/mcp-tools'] },
        { label: 'Project', items: ['comparison', 'developer', 'appendices', 'documentation-plan'] },
        { label: 'Rust API', link: '/api/' },
      ],
    }),
  ],
});
