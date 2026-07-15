import type { Metadata } from "next";
import { GeistMono } from "geist/font/mono";
import { GeistSans } from "geist/font/sans";
import { SiteNav } from "@/components/SiteNav";
import "./globals.css";

export const metadata: Metadata = {
  metadataBase: new URL("https://tracefold.vercel.app"),
  title: {
    default: "TraceFold — Query-Preserving Telemetry Archives",
    template: "%s · TraceFold",
  },
  description:
    "An executable research artifact for query-preserving compression of tiered telemetry archives.",
  openGraph: {
    title: "TraceFold",
    description: "Query-preserving compression for tiered telemetry archives.",
    type: "website",
    images: ["/opengraph.svg"],
  },
};

export default function RootLayout({ children }: Readonly<{ children: React.ReactNode }>) {
  return (
    <html
      lang="en"
      className={`${GeistSans.variable} ${GeistMono.variable}`}
      data-scroll-behavior="smooth"
    >
      <body>
        <SiteNav />
        <main>{children}</main>
        <footer className="site-footer">
          <p>TraceFold · nearlynamed · Technical report, not peer reviewed</p>
          <a href="https://github.com/nearlynamed/tracefold">Source and raw evidence</a>
        </footer>
      </body>
    </html>
  );
}
