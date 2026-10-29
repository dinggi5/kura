// 캐러셀을 장마다 PNG 로 (개발 81).
// 사용: PLAYWRIGHT=<playwright-core 경로> node promo/render.cjs [출력 폴더]
// 브라우저는 깔린 Google Chrome 을 쓴다(따로 받지 않는다). 폰트는 CDN(Pretendard)이라 인터넷이 필요하다.
const path = require("path");
const { chromium } = require(process.env.PLAYWRIGHT || "playwright-core");

(async () => {
  const out = path.resolve(process.argv[2] || path.join(__dirname, "carousel", "out"));
  require("fs").mkdirSync(out, { recursive: true });
  const browser = await chromium.launch({ executablePath: "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" });
  const page = await browser.newPage({ viewport: { width: 1200, height: 1500 }, deviceScaleFactor: 1, colorScheme: "light" });
  await page.goto("file://" + path.join(__dirname, "carousel", "index.html"), { waitUntil: "networkidle" });
  await page.evaluate(() => document.fonts.ready);
  const slides = await page.$$(".slide");
  for (const [i, el] of slides.entries()) {
    await el.screenshot({ path: path.join(out, `kura-${String(i + 1).padStart(2, "0")}.png`) });
  }
  await browser.close();
  console.log(`${slides.length}장 → ${out}`);
})().catch((e) => { console.error(e); process.exit(1); });
