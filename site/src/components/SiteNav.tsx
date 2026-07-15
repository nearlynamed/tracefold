import Link from "next/link";

const links = [
  ["Research", "/"],
  ["Paper", "/paper/"],
  ["Data", "/data/"],
  ["Reproduce", "/reproduce/"],
] as const;

export function SiteNav() {
  return (
    <header className="site-header">
      <nav className="site-nav" aria-label="Primary navigation">
        <Link className="wordmark" href="/">
          TraceFold
        </Link>
        <div className="nav-links">
          {links.map(([label, href]) => (
            <Link key={href} href={href}>
              {label}
            </Link>
          ))}
          <a href="https://github.com/nearlynamed/tracefold">GitHub</a>
        </div>
      </nav>
    </header>
  );
}
