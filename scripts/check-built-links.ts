import { readFile, readdir, stat } from 'node:fs/promises';
import { join } from 'node:path';

async function files(directory: string): Promise<string[]> {
  const output: string[] = [];
  for (const name of await readdir(directory)) {
    const path = join(directory, name);
    if ((await stat(path)).isDirectory()) output.push(...await files(path));
    else output.push(path);
  }
  return output;
}

const root = 'dist';
const all = await files(root);
const known = new Set(all.map((path) => path.replaceAll('\\', '/')));
const missing: string[] = [];

const pages = all.filter((path) => path.endsWith('.html') && !path.replaceAll('\\', '/').startsWith('dist/api/'));
for (const path of pages) {
  const html = await readFile(path, 'utf8');
  const page = `https://docs.invalid/editor/${path.replaceAll('\\', '/').slice(root.length + 1)}`;
  for (const match of html.matchAll(/(?:href|src)="([^"]+)"/g)) {
    const value = match[1];
    if (value.startsWith('#') || value.startsWith('data:') || value.startsWith('javascript:')) continue;
    const url = new URL(value, page);
    if (url.origin !== 'https://docs.invalid' || !url.pathname.startsWith('/editor/')) continue;
    let target = `${root}/${decodeURIComponent(url.pathname.slice('/editor/'.length))}`.replaceAll('\\', '/');
    if (target.endsWith('/')) target += 'index.html';
    if (!known.has(target)) missing.push(`${path}: ${value}`);
  }
}

if (missing.length) {
  throw new Error(`Broken built links:\n${missing.join('\n')}`);
}
console.log(`checked local links in ${pages.length} Starlight HTML files`);
