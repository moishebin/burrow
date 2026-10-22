#!/usr/bin/env node
// Tiny DevTools Protocol client for a Burrow started with --remote-debugging-port=9222 (dev only).
// Linux/WSL counterpart of cdp.ps1.
//
//   node scripts/cdp.mjs list
//   node scripts/cdp.mjs <url-substring> eval "<js>"
//   node scripts/cdp.mjs <url-substring> shot out.png
import { writeFileSync } from "node:fs";

const [, , match, action, arg] = process.argv;
const port = process.env.CDP_PORT || 9222;
const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
if (match === "list") {
  for (const t of targets) console.log(`${t.type}\t${t.title}\t${t.url}`);
  process.exit(0);
}
const target = targets.find((t) => t.type === "page" && t.url.includes(match));
if (!target) {
  console.error(`no page matching ${match}`);
  process.exit(1);
}

const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((r) => ws.addEventListener("open", r, { once: true }));
let next = 1;
const call = (method, params = {}) =>
  new Promise((resolve) => {
    const id = next++;
    const onMessage = (ev) => {
      const msg = JSON.parse(ev.data);
      if (msg.id !== id) return;
      ws.removeEventListener("message", onMessage);
      resolve(msg);
    };
    ws.addEventListener("message", onMessage);
    ws.send(JSON.stringify({ id, method, params }));
  });

if (action === "eval") {
  const r = await call("Runtime.evaluate", { expression: arg, returnByValue: true, awaitPromise: true, userGesture: true });
  const res = r.result;
  console.log(res.exceptionDetails ? res.exceptionDetails.exception?.description : JSON.stringify(res.result.value));
} else if (action === "shot") {
  const r = await call("Page.captureScreenshot", { format: "png" });
  writeFileSync(arg, Buffer.from(r.result.data, "base64"));
  console.log(`saved ${arg}`);
}
ws.close();
