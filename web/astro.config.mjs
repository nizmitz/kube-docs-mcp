import { defineConfig } from "astro/config";

export default defineConfig({
  site: "https://kubedocs.nizmitz.com",
  output: "static",
  build: { inlineStylesheets: "always" },
  compressHTML: true,
});
