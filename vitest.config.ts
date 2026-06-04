import { defineConfig } from "vitest/config";
import solid from "vite-plugin-solid";

export default defineConfig({
  plugins: [solid()],
  resolve: { alias: { "~": new URL("./src", import.meta.url).pathname } },
  test: {
    environment: "jsdom",
    globals: true,
  },
});
