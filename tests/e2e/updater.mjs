// Updater rehearsal with two signed TEST builds (dev key, local HTTP feed).
// Scenarios, in order, against an installed 0.1.0-test:
//   offline (feed down) -> invalid feed -> already up to date -> wrong signature ->
//   interrupted download -> valid update to 0.1.1 (installer, relaunch, data preserved).
//
// Usage:
//   node tests/e2e/updater.mjs --v010 <dir with 0.1.0 setup.exe> --v011 <dir with 0.1.1 setup.exe + .sig> --data <dir> --shots <dir>
import { chromium } from "playwright-core";
import { spawn, execFileSync } from "node:child_process";
import { createServer } from "node:http";
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync } from "node:fs";
import path from "node:path";

const arg = (n, d) => {
  const i = process.argv.indexOf(`--${n}`);
  return i > 0 ? process.argv[i + 1] : d;
};
const v010 = path.resolve(arg("v010", "out/updater-test/v0.1.0"));
const v011 = path.resolve(arg("v011", "out/updater-test/v0.1.1"));
const data = path.resolve(arg("data", "out/updater-e2e-data"));
const shots = path.resolve(arg("shots", "out/updater-e2e-shots"));
const port = 9334;
const FEED_PORT = 8765;
const log = (...a) => console.log("[upd]", ...a);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const setup = (dir) => path.join(dir, readdirSync(dir).find((f) => f.endsWith("-setup.exe")));
const setup010 = setup(v010);
const setup011 = setup(v011);
const sig011 = readFileSync(setup011 + ".sig", "utf8").trim();
const sig010 = readFileSync(setup010 + ".sig", "utf8").trim();
const installer011 = readFileSync(setup011);

// ---------- local feed ----------
let mode = "down";
const manifest = (version, signature) =>
  JSON.stringify({
    version,
    notes: `Versão de teste ${version} (ensaio do updater)`,
    pub_date: "2026-11-20T12:00:00Z",
    platforms: { "windows-x86_64": { signature, url: `http://127.0.0.1:${FEED_PORT}/setup-${version}.exe` } },
  });
const hits = [];
const server = createServer((req, res) => {
  hits.push(`${mode} ${req.method} ${req.url}`);
  if (req.url === "/latest.json") {
    if (mode === "garbage") return res.end("isto não é json {");
    if (mode === "uptodate") return res.end(manifest("0.1.0", sig010));
    if (mode === "badsig") return res.end(manifest("0.1.1", sig010)); // signature of another file
    return res.end(manifest("0.1.1", sig011));
  }
  if (req.url === "/setup-0.1.1.exe") {
    res.writeHead(200, { "Content-Type": "application/octet-stream", "Content-Length": installer011.length });
    if (mode === "truncated") {
      res.write(installer011.subarray(0, Math.floor(installer011.length / 3)));
      setTimeout(() => res.socket.destroy(), 100);
      return;
    }
    return res.end(installer011);
  }
  res.writeHead(404).end();
});
const listen = () => new Promise((r) => server.listen(FEED_PORT, "127.0.0.1", r));
const unlisten = () => new Promise((r) => server.close(r));

// ---------- app helpers ----------
const installDir = () => {
  const base = process.env.LOCALAPPDATA;
  const d = readdirSync(base).find((n) => n.startsWith("PokerLab UpdaterTest"));
  return d ? path.join(base, d) : null;
};
const appExe = () => {
  const d = installDir();
  if (!d) return null;
  const f = readdirSync(d).find((n) => n.endsWith(".exe") && !n.startsWith("uninstall"));
  return path.join(d, f);
};

function launch() {
  return spawn(appExe(), [], {
    env: { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`, POKERLAB_DATA_DIR: data },
    stdio: "ignore",
  });
}
async function connect(timeoutMs = 45000) {
  const t = Date.now();
  while (Date.now() - t < timeoutMs) {
    try {
      const browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
      for (let j = 0; j < 40; j++) {
        const page = browser.contexts().flatMap((c) => c.pages()).find((p) => !p.url().startsWith("devtools"));
        if (page) return { browser, page };
        await sleep(250);
      }
      await browser.close();
    } catch {
      /* retry */
    }
    await sleep(500);
  }
  throw new Error("DevTools não respondeu");
}
function killAll() {
  try {
    execFileSync("taskkill", ["/IM", path.basename(appExe() ?? "pokerlab.exe"), "/T"], { stdio: "ignore" });
  } catch {
    /* none */
  }
}
async function closeApp(child) {
  try {
    execFileSync("taskkill", ["/PID", String(child.pid), "/T"], { stdio: "ignore" });
  } catch {
    /* gone */
  }
  for (let i = 0; i < 50 && child.exitCode === null; i++) await sleep(200);
  if (child.exitCode === null) execFileSync("taskkill", ["/PID", String(child.pid), "/T", "/F"], { stdio: "ignore" });
  await sleep(500);
}
async function launcherText(page) {
  return (await page.locator(".launcher-box").innerText().catch(() => "")).replace(/\n+/g, " ");
}
async function version(page) {
  return (await page.locator(".sidebar-foot").innerText()).trim();
}

const results = [];
const check = (name, cond, detail) => {
  results.push({ name, ok: !!cond, detail });
  log(`${cond ? "OK  " : "FALHA"} ${name} — ${detail}`);
};

// ---------- 0. install 0.1.0-test ----------
rmSync(data, { recursive: true, force: true });
mkdirSync(data, { recursive: true });
mkdirSync(shots, { recursive: true });
killAll();
execFileSync(setup010, ["/S"]);
check("instalar 0.1.0-test (NSIS por usuário, silencioso)", existsSync(appExe() ?? ""), appExe());

// ---------- 1. offline ----------
{
  mode = "down";
  const child = launch();
  const { browser, page } = await connect();
  await page.getByText(/Sem atualização agora|tempo limite/).waitFor({ timeout: 20000 });
  const t = await launcherText(page);
  await page.getByRole("heading", { name: "Início" }).waitFor({ timeout: 20000 });
  check("sem internet: abre o treino", true, t);
  // Create history to be preserved by the update.
  await page.getByText("Bem-vindo!").waitFor();
  await page.getByRole("button", { name: /Padrão — 20 decisões/ }).click();
  await page.getByRole("button", { name: "Treinar", exact: true }).first().click();
  await page.getByLabel("Número de decisões").fill("2");
  await page.getByRole("button", { name: "Começar" }).click();
  for (let i = 0; i < 2; i++) {
    await page.locator(".actions .action").first().waitFor();
    await page.keyboard.press("2");
    await page.locator(".feedback").waitFor();
    await page.keyboard.press("Enter");
  }
  await page.getByRole("heading", { name: "Resumo da sessão" }).waitFor();
  check("histórico criado na 0.1.0", (await version(page)).includes("v0.1.0"), await version(page));
  await browser.close();
  await closeApp(child);
}

await listen();

// ---------- 2. invalid feed ----------
for (const [m, expect, name] of [
  ["garbage", /Sem atualização agora/, "feed inválido: mostra erro e segue offline"],
  ["uptodate", /versão mais recente/, "já atualizado"],
]) {
  mode = m;
  const child = launch();
  const { browser, page } = await connect();
  await page.getByText(expect).waitFor({ timeout: 20000 });
  const t = await launcherText(page);
  await page.getByRole("heading", { name: "Início" }).waitFor({ timeout: 20000 });
  check(name, true, t);
  await browser.close();
  await closeApp(child);
}

// ---------- 3. bad signature / interrupted download ----------
for (const [m, name] of [
  ["badsig", "assinatura incorreta: recusa instalar e mantém a versão"],
  ["truncated", "download interrompido: mostra erro e mantém a versão"],
]) {
  mode = m;
  const child = launch();
  const { browser, page } = await connect();
  await page.getByRole("heading", { name: /Nova versão 0\.1\.1/ }).waitFor({ timeout: 20000 });
  await page.getByRole("button", { name: "Atualizar agora" }).click();
  await page.getByText("A atualização falhou").waitFor({ timeout: 60000 });
  const t = await launcherText(page);
  await page.screenshot({ path: path.join(shots, `falha-${m}.png`) });
  await page.getByRole("button", { name: "Continuar" }).click();
  await page.getByRole("heading", { name: "Início" }).waitFor();
  const v = await version(page);
  check(name, v.includes("v0.1.0"), `${t} | ${v}`);
  await browser.close();
  await closeApp(child);
}

// ---------- 4. valid update ----------
{
  mode = "valid";
  const child = launch();
  let { browser, page } = await connect();
  await page.getByRole("heading", { name: /Nova versão 0\.1\.1/ }).waitFor({ timeout: 20000 });
  await page.screenshot({ path: path.join(shots, "atualizacao-disponivel.png") });
  const t0 = Date.now();
  await page.getByRole("button", { name: "Atualizar agora" }).click();
  // The app exits, the NSIS installer runs in passive mode and relaunches the app.
  await browser.close().catch(() => undefined);
  for (let i = 0; i < 100 && child.exitCode === null; i++) await sleep(300);
  check("app encerra para o instalador", child.exitCode !== null, `exit=${child.exitCode}`);
  let relaunched = null;
  try {
    relaunched = await connect(120000);
  } catch {
    /* relaunched without devtools env? start manually below */
  }
  let manual = null;
  if (!relaunched) {
    log("o app relançado não expôs DevTools; abrindo manualmente para verificar");
    manual = launch();
    relaunched = await connect();
  }
  ({ browser, page } = relaunched);
  await page.getByRole("heading", { name: "Início" }).waitFor({ timeout: 30000 });
  const v = await version(page);
  check("relança na 0.1.1 após instalar", v.includes("v0.1.1"), `${v} em ${Math.round((Date.now() - t0) / 1000)} s; relançado pelo instalador: ${!manual}`);
  const banner = await page.locator(".alert-ok").innerText().catch(() => "");
  check("verificação de saúde após atualizar", /Atualizado de 0\.1\.0 para 0\.1\.1/.test(banner), banner);
  await page.screenshot({ path: path.join(shots, "apos-atualizar.png") });
  await page.getByRole("button", { name: "Progresso", exact: true }).first().click();
  await page.locator("table.history tbody tr").first().waitFor();
  const n = await page.locator(".stat .stat-v").first().innerText();
  check("histórico preservado", Number(n) === 2, `n=${n}`);
  await browser.close();
  killAll();
}

await unlisten();
log("requisições ao feed:", hits.length);
const failed = results.filter((r) => !r.ok);
console.log(failed.length ? `[upd] FALHOU (${failed.length})` : "[upd] OK — todos os cenários");
process.exit(failed.length ? 1 : 0);
