import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },

  // 라이브러리를 앱 코드와 다른 청크로 (개발 71 — 개발 63 부터 이월된 「청크 507kB」 경고). 로컬 앱이라 크기 자체는
  // 디스크에서 읽는 값이지만, 경고가 늘 떠 있으면 진짜로 커진 날을 못 알아본다. 화면을 지연 로드(dynamic import)하진
  // 않는다 — 승인 창이 뜨는 순간 한 박자 늦게 그려지면 안 된다.
  build: {
    rollupOptions: {
      output: {
        manualChunks: {
          react: ["react", "react-dom"],
          motion: ["framer-motion"],
        },
      },
    },
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
}));
