import { defineConfig } from "vitest/config";

export default defineConfig({
  clearScreen: false,
  test: {
    include: ["frontend/**/*.test.ts"],
  },
  server: {
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  build: {
    outDir: "dist-ui",
    emptyOutDir: true,
    target: "chrome105",
  },
});
