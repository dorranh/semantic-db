import { readdir, readFile, stat } from 'node:fs/promises';
import { resolve } from 'node:path';
import { base, sourceBase } from './documentation-links.mjs';

const output = resolve('dist');
async function files(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  return (await Promise.all(entries.map(entry => entry.isDirectory()
    ? files(resolve(directory, entry.name)) : resolve(directory, entry.name)))).flat();
}
const pages = (await files(output)).filter(path => path.endsWith('.html'));
const errors = [];
for (const page of pages) {
  const html = await readFile(page, 'utf8');
  for (const [, href] of html.matchAll(/(?:href|src)="([^"]+)"/g)) {
    if (href.startsWith(sourceBase)) {
      const source = decodeURI(href.slice(sourceBase.length).split(/[?#]/)[0]);
      try { await stat(resolve('../..', source)); }
      catch { errors.push(`${page}: missing repository file ${source}`); }
      continue;
    }
    if (/^(?:[a-z][a-z\d+.-]*:|\/\/)/i.test(href)) continue;
    const [path, hash] = href.split('#');
    let target = page;
    if (path) {
      if (!path.startsWith(`${base}/`)) { errors.push(`${page}: missing base in ${href}`); continue; }
      target = resolve(output, decodeURI(path.slice(base.length + 1)).split('?')[0]);
      try { if ((await stat(target)).isDirectory()) target = resolve(target, 'index.html'); }
      catch { errors.push(`${page}: missing ${href}`); continue; }
    }
    try {
      const body = await readFile(target, 'utf8');
      if (hash && !body.includes(`id="${decodeURIComponent(hash)}"`)) errors.push(`${page}: missing anchor ${href}`);
    } catch { errors.push(`${page}: missing ${href}`); }
  }
}
if (errors.length) { console.error(errors.join('\n')); process.exit(1); }
console.log(`Checked internal links, assets, and anchors in ${pages.length} pages.`);
