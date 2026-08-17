import { mergeConfig } from "vite";
import { defineConfig } from "vitest/config";

import viteConfig from "./vite.config.ts";

export default mergeConfig(
  viteConfig,
  defineConfig({
    test: {
      fileParallelism: true,
      hookTimeout: 10_000,
      isolate: true,
      maxConcurrency: 5,
      maxWorkers: 4,
      pool: "forks",
      retry: 0,
      sequence: {
        seed: 20_260_817,
        shuffle: {
          files: false,
          tests: false,
        },
      },
      slowTestThreshold: 1_000,
      teardownTimeout: 10_000,
      testTimeout: 10_000,
    },
  }),
);
