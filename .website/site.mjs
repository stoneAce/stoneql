export function siteBase(value = process.env.DOCS_BASE || '/') {
  if (!/^\/(?:[A-Za-z0-9._-]+\/)*$/.test(value) || value.split('/').some(part => part === '.' || part === '..')) throw new Error('DOCS_BASE must be a slash-delimited absolute path');
  return value;
}
export function repositoryUrl() {
  const value = process.env.DOCS_REPOSITORY_URL || (process.env.GITHUB_REPOSITORY ? `https://github.com/${process.env.GITHUB_REPOSITORY}` : '');
  if (value && !/^https:\/\/github\.com\/[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(value)) throw new Error('Expected a public GitHub repository URL');
  return value;
}
export const releaseUrl = 'https://github.com/stoneAce/stoneql/releases/tag/v0.1.1';
export const guideSidebar = [
  { text: 'User guide', items: [
    { text: 'Overview', link: '/guide/' },
    { text: 'Download', link: '/guide/download' },
    { text: 'Install', link: '/guide/install' },
    { text: 'Connect to a database', link: '/guide/connect' },
    { text: 'Browse and edit tables', link: '/guide/browse-and-edit' },
    { text: 'Run SQL', link: '/guide/run-sql' },
    { text: 'Import and export', link: '/guide/import-export' },
    { text: 'Troubleshooting', link: '/guide/troubleshooting' },
    { text: "What's in this release", link: '/guide/beta' }
  ] }
];
const title = name => name.split('-').map(word => word[0].toUpperCase() + word.slice(1)).join(' ');
export const developerSidebar = [
  { text: 'Overview and contributing', items: [
    { text: 'Developer Docs', link: '/development/' },
    { text: 'Contributor setup', link: '/development/contributor-setup' },
    { text: 'Browser demo', link: '/development/browser-demo' },
    { text: 'Documentation', link: '/development/documentation' },
    { text: 'Branding', link: '/development/branding' }
  ] },
  { text: 'Architecture', items: ['overview', 'database-adapters', 'connection-management', 'api-contract', 'data-integrity', 'import-export', 'security', 'deployment', 'driver-evaluation'].map(name => ({ text: title(name), link: `/architecture/${name}` })) },
  { text: 'Development', items: [
    { text: 'Rust Foundation', link: '/development/rust-foundation' },
    { text: 'Desktop Host', link: '/development/desktop-host' },
    { text: 'Connection Lifecycle', link: '/development/connection-lifecycle' },
    { text: 'Read-only SQL Safety', link: '/development/read-only-safety' },
    { text: 'Read-write SQL Execution', link: '/development/sql-execution' },
    { text: 'PostgreSQL & SQLite', link: '/development/postgres-sqlite' },
    { text: 'SSH & Credentials', link: '/development/ssh-credentials' },
    { text: 'Resource Profiles', link: '/development/resource-profiles' },
    { text: 'Demo Database Objects', link: '/database-objects' },
    { text: 'Prototype Reference', link: '/development/prototype-reference' }
  ] },
  { text: 'Milestones & verification', items: [
    { text: 'Roadmap', link: '/architecture/roadmap' },
    { text: 'Verification', link: '/development/verification' },
    { text: 'UX Review', link: '/ux-review-remediation' }
  ] }
];
// VitePress multi-sidebar: the longest matching path prefix selects the sidebar.
export const sidebar = {
  '/guide/': guideSidebar,
  '/development/': developerSidebar,
  '/architecture/': developerSidebar,
  '/database-objects': developerSidebar,
  '/ux-review-remediation': developerSidebar
};
// Every navigation group, for link checks.
export const sidebarGroups = [...guideSidebar, ...developerSidebar];
