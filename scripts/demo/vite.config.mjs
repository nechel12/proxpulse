// `npm run demo`: the real frontend with the demo backend loaded before it.
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../..", import.meta.url));

export default {
  root,
  clearScreen: false,
  server: { port: 1430, strictPort: true, host: "127.0.0.1" },
  plugins: [
    {
      name: "demo-backend",
      transformIndexHtml: () => [
        { tag: "script", attrs: { type: "module", src: "/scripts/demo/mock-tauri.js" }, injectTo: "head-prepend" },
      ],
    },
  ],
};
