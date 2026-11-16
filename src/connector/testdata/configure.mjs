// Configures the Oido Browser Connector in a running Chromium over CDP:
// node configure.mjs <profile dir> <base> <token> <allowed origin>
import fs from "node:fs";
const [profile, base, token, origin] = process.argv.slice(2);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let port;
for (let i = 0; i < 100 && !port; i++) {
  try { port = fs.readFileSync(`${profile}/DevToolsActivePort`, "utf8").split("\n")[0]; } catch { await sleep(100); }
}
let sw;
for (let i = 0; i < 100 && !sw; i++) {
  const list = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json().catch(() => []);
  sw = list.find((t) => t.type === "service_worker" && t.url.endsWith("/background.js"));
  if (!sw) await sleep(200);
}
if (!sw) { console.error("no connector service worker"); process.exit(2); }
const ws = new WebSocket(sw.webSocketDebuggerUrl);
await new Promise((r) => ws.addEventListener("open", r));
const expr = `(async () => {
  await chrome.storage.local.set({ base: ${JSON.stringify(base)}, token: ${JSON.stringify(token)}, allowlist: [${JSON.stringify(origin)}], allowAll: false, disabled: false });
  if (await chrome.offscreen.hasDocument()) await chrome.offscreen.closeDocument();
  await chrome.offscreen.createDocument({ url: "offscreen.html", reasons: ["WORKERS"], justification: "test" });
  return "configured";
})()`;
ws.send(JSON.stringify({ id: 1, method: "Runtime.evaluate", params: { expression: expr, awaitPromise: true } }));
ws.addEventListener("message", (e) => {
  const m = JSON.parse(e.data);
  if (m.id === 1) { console.log(JSON.stringify(m.result?.result?.value ?? m)); process.exit(0); }
});
setTimeout(() => { console.error("timeout"); process.exit(3); }, 10000);
