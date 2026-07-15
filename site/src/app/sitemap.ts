import type { MetadataRoute } from "next";

export const dynamic = "force-static";

export default function sitemap(): MetadataRoute.Sitemap {
  return ["", "/paper/", "/data/", "/reproduce/"].map((path) => ({
    url: `https://tracefold.vercel.app${path}`,
    changeFrequency: "monthly",
    priority: path ? 0.7 : 1,
  }));
}
