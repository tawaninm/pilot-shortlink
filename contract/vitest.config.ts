import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    include: ["test/**/*.test.ts"],
    testTimeout: 30_000,
    // Backends on free tiers: keep requests sequential across files so rate limits and CPU stay predictable
    fileParallelism: false,
  },
});
