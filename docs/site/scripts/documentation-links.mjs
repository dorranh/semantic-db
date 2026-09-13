import { existsSync } from 'node:fs';
import { dirname, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../../', import.meta.url));
const repository = resolve(root, '..');
export const base = '/semantic-db';
export const sourceBase = 'https://github.com/dorranh/semantic-db/blob/main/';
export const pageId = (path) => path.replace(/\.md$/i, '').toLowerCase();
export const pageUrl = (id) => id === 'generated/site-home' ? `${base}/` : `${base}/${id}/`;

// Preserve source Markdown; resolve links for the deployed subdirectory at render time.
export function documentationLinks() {
  return (tree, file) => {
    function visit(node) {
      if (node.type === 'code' && node.lang?.startsWith('rust,')) node.lang = 'rust';
      if (['link', 'definition', 'image'].includes(node.type) && node.url &&
          !/^(?:[a-z][a-z\d+.-]*:|\/\/|#)/i.test(node.url)) {
        let [path, suffix = ''] = node.url.split(/(?=[?#])/s, 2);
        const line = path.match(/:(\d+)$/);
        if (line) { path = path.slice(0, -line[0].length); suffix = `#L${line[1]}`; }
        // Old assessment links contain author-machine paths; keep builds portable.
        if (path.startsWith('/Users/') && path.includes('/semantic-db/')) {
          path = resolve(repository, path.split('/semantic-db/')[1]);
        }
        let target = resolve(dirname(file.path), decodeURI(path));
        if (!existsSync(target) && file.path.includes('/generated/spikes/')) {
          const beforeMove = resolve(dirname(dirname(file.path)), decodeURI(path));
          if (existsSync(beforeMove)) target = beforeMove;
        }
        // These assessments were moved without changing their original Markdown.
        if (!existsSync(target) && target.startsWith(resolve(root, 'generated') + sep)) {
          const moved = resolve(root, 'generated/spikes', target.split(sep).at(-1));
          if (existsSync(moved)) target = moved;
        }
        const docPath = relative(root, target).split(sep).join('/');
        if (!docPath.startsWith('../') && /\.md$/i.test(docPath) && existsSync(target)) {
          node.url = pageUrl(pageId(docPath)) + suffix;
        } else {
          const repoPath = relative(repository, target).split(sep).map(encodeURIComponent).join('/');
          node.url = sourceBase + repoPath + suffix;
        }
      }
      node.children?.forEach(visit);
    }
    visit(tree);
  };
}
