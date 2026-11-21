// End-to-end test of the full-hand simulation in the real app (real pack, real engine, real
// SQLite), driven through the WebView2 DevTools protocol. Flow: open "Mão completa" -> solve a
// flop tree in the background (progress visible, UI responsive) -> play hands to the end ->
// history -> close -> reopen -> history persisted.
//
// Usage:
//   node tests/e2e/hand-sim.mjs --exe <pokerlab.exe> --pack src-tauri/resources/packs/btn-vs-bb-srp-100bb \
//        --data out/e2e-hand-data --shots out/e2e-hand-shots --board 9h6h3h --hands 3 --hero BB
import { chromium } from "playwright-core";
import { spawn, execFileSync } from "node:child_process";
import { mkdirSync, openSync, rmSync } from "node:fs";
import path from "node:path";

const arg = (name, def) => {
  const i = process.argv.indexOf(`--${name}`);
  return i > 0 ? process.argv[i + 1] : def;
};
const exe = arg("exe", path.join(process.env.LOCALAPPDATA, "PokerLab", "pokerlab.exe"));
const pack = arg("pack", "");
const data = path.resolve(arg("data", "out/e2e-hand-data"));
const shots = path.resolve(arg("shots", "out/e2e-hand-shots"));
const port = Number(arg("port", "9334"));
const board = arg("board", "9h6h3h");
const hands = Number(arg("hands", "3"));
const hero = arg("hero", "BB"); // BB | BTN | random
const HERO_LABEL = { BB: "BB (fora de posição)", BTN: "BTN (em posição)", random: "Aleatória" };

rmSync(data, { recursive: true, force: true });
mkdirSync(data, { recursive: true });
mkdirSync(shots, { recursive: true });

const log = (...a) => console.log("[e2e]", ...a);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function launch() {
  const env = { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`, POKERLAB_DATA_DIR: data };
  if (pack) env.POKERLAB_PACK_DIR = path.resolve(pack);
  const errLog = openSync(path.join(data, "..", `app-stderr-${port}.log`), "a");
  return spawn(exe, [], { env, stdio: ["ignore", errLog, errLog] });
}

async function connect() {
  for (let i = 0; i < 60; i++) {
    try {
      const browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
      for (let j = 0; j < 40; j++) {
        const page = browser.contexts().flatMap((c) => c.pages()).find((p) => !p.url().startsWith("devtools"));
        if (page) return { browser, page };
        await sleep(250);
      }
    } catch {
      /* not up yet */
    }
    await sleep(500);
  }
  throw new Error("WebView2 DevTools endpoint did not come up");
}

async function shutdown(child) {
  try {
    execFileSync("taskkill", ["/PID", String(child.pid), "/T"], { stdio: "ignore" });
  } catch {
    /* gone */
  }
  const t = Date.now();
  while (child.exitCode === null && Date.now() - t < 15000) await sleep(200);
  if (child.exitCode === null) execFileSync("taskkill", ["/PID", String(child.pid), "/T", "/F"], { stdio: "ignore" });
}

let ok = true;
const fail = (m) => {
  ok = false;
  console.error("[e2e] FALHA:", m);
};

// ---------- Run 1 ----------
let child = launch();
let { browser, page } = await connect();
await page.getByRole("heading", { name: "Início" }).waitFor({ timeout: 60000 });
await page.getByRole("button", { name: "Mão completa", exact: true }).first().click();
await page.getByRole("heading", { name: "Mão completa" }).waitFor();
await page.getByLabel("Board (flop)").selectOption({ label: board });
await page.getByLabel(HERO_LABEL[hero]).check();
await page.getByRole("button", { name: `Resolver ${board}` }).click();
await page.getByText(/Resolvendo a árvore completa/).waitFor({ timeout: 10000 });
await sleep(3000);
await page.screenshot({ path: path.join(shots, "01-resolvendo.png") });
// UI stays responsive while solving: a status poll answered and the sidebar works.
const t0 = Date.now();
const progressText = await page.locator("[role=status] p").first().innerText();
log(`durante o solve: "${progressText}" (lido em ${Date.now() - t0} ms)`);
const solveStart = Date.now();
await page.getByRole("button", { name: `Jogar mão no board ${board}` }).waitFor({ timeout: 600000 });
const solveMs = Date.now() - solveStart;
const readyText = await page.locator(".card-panel", { hasText: "pronto" }).innerText();
log(`pronto em ~${Math.round(solveMs / 1000)} s: ${readyText.replace(/\n/g, " ")}`);
if (!readyText.includes("mesma configuração do pacote")) fail("configuração diferente da do pacote");
await page.screenshot({ path: path.join(shots, "02-pronto.png") });

const played = [];
await page.getByRole("button", { name: `Jogar mão no board ${board}` }).click();
for (let h = 0; h < hands; h++) {
  if (h > 0) await page.keyboard.press("Enter");
  let steps = 0;
  const villainHiddenAtStart = (await page.locator(".card-back").count()) === 2;
  for (;;) {
    await page.waitForFunction(() => document.querySelector(".actions .action") || document.querySelector(".result-hero, .result-villain, .result-split"));
    if ((await page.locator(".actions .action").count()) === 0) break;
    const labels = await page.locator(".actions .action").allInnerTexts();
    // Avoid folding so hands reach later streets; alternate passive/aggressive.
    const choices = labels.map((l, i) => [l, i]).filter(([l]) => !/Desistir/.test(l));
    const pick = choices[(h + steps) % choices.length][1];
    await page.locator(".actions .action").nth(pick).click();
    steps++;
    await page.waitForFunction((n) => {
      const d = document.querySelectorAll(".feedback").length;
      return d > 0 || document.querySelector(".result-hero, .result-villain, .result-split") || n === 0;
    }, steps);
    if (h === 0 && steps === 1) await page.screenshot({ path: path.join(shots, "03-decisao-e-nota.png") });
    if (steps > 12) {
      fail("mão não terminou");
      break;
    }
  }
  await page.locator(".result-hero, .result-villain, .result-split").first().waitFor();
  const result = await page.locator(".result-hero, .result-villain, .result-split").first().innerText();
  const villainShown = (await page.locator(".villain .card:not(.card-back)").count()) === 2;
  const boardCards = await page.locator(".board .card").count();
  const reviews = await page.locator(".decision-review summary").allInnerTexts();
  const log_ = await page.locator(".hand-log").innerText();
  const potShown = (await page.locator(".board .pot b").first().innerText()).trim();
  if (!result.includes(`pote de ${potShown}`)) fail(`pote na mesa (${potShown}) difere do pote disputado`);
  played.push({ result: result.split("\n")[0], steps, boardCards, reviews });
  log(`mão ${h + 1}: ${result.split("\n")[0]} | ${steps} decisões | board ${boardCards} cartas`);
  log(`  ação: ${log_.replace(/\n/g, " / ")}`);
  for (const r of reviews) log(`  - ${r}`);
  if (!villainHiddenAtStart) fail("cartas do adversário visíveis durante a mão");
  if (!villainShown) fail("cartas do adversário não reveladas no fim");
  if (reviews.length !== steps) fail(`revisão lista ${reviews.length} de ${steps} decisões`);
  await page.screenshot({ path: path.join(shots, `04-fim-mao-${h + 1}.png`), fullPage: true });
}
await page.getByRole("button", { name: "Voltar" }).click();
await page.getByRole("heading", { name: "Histórico de mãos" }).waitFor();
const rows1 = await page.locator("table tbody tr").count();
await page.screenshot({ path: path.join(shots, "05-historico.png"), fullPage: true });
if (rows1 !== hands) fail(`histórico com ${rows1} linhas (esperado ${hands})`);
await browser.close();
await shutdown(child);
log("app fechado");

// ---------- Run 2: reopen ----------
child = launch();
({ browser, page } = await connect());
await page.getByRole("heading", { name: "Início" }).waitFor({ timeout: 60000 });
await page.getByRole("button", { name: "Mão completa", exact: true }).first().click();
await page.getByRole("heading", { name: "Histórico de mãos" }).waitFor();
await page.locator("table tbody tr").first().waitFor();
const rows2 = await page.locator("table tbody tr").allInnerTexts();
const finished = await page.locator(".stat .stat-v").first().innerText();
await page.screenshot({ path: path.join(shots, "06-historico-apos-reabrir.png"), fullPage: true });
await browser.close();
await shutdown(child);

if (rows2.length !== hands) fail(`após reabrir: ${rows2.length} linhas`);
if (Number(finished) !== hands) fail(`após reabrir: ${finished} mãos concluídas`);
log("histórico após reabrir:\n  " + rows2.map((r) => r.replace(/\t/g, " | ")).join("\n  "));
console.log(ok ? "[e2e] OK" : "[e2e] FALHOU");
process.exit(ok ? 0 : 1);
