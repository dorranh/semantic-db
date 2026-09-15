export const groups = [
  { title: 'Use Semantic DB', entries: [
    ['adding-datasets', 'Add a dataset'],
    ['cli', 'CLI and REPL'],
    ['embedding', 'Embed in Rust'],
    ['building-connectors', 'Build a connector'],
    ['connectors', 'Configuration'],
    ['ossie-reference', 'Ossie reference'],
  ] },
  { title: 'Generated guides', entries: [
    ['generated/release-quickstart', 'Start from a release'],
    ['generated/file-connectors', 'Built-in connectors'],
    ['generated/delivery', 'Delivery and integration'],
    ['generated/supported-types', 'Types and extensibility'],
    ['generated/database-comparison', 'Database comparison'],
    ['generated/performance', 'Performance and federation'],
    ['generated/docs-site', 'Maintain this site'],
  ] },
];
export function titleOf(entry: { body?: string; id: string }) {
  return entry.body?.match(/^#\s+(.+)$/m)?.[1] ?? entry.id.split('/').at(-1) ?? entry.id;
}
