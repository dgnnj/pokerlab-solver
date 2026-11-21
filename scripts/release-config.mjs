// Prints a Tauri config overlay for production releases: real GitHub Releases endpoint and the
// production updater public key. The dev key in tauri.conf.json is never used for releases.
const [repo, pubkey] = process.argv.slice(2);
if (!/^[\w.-]+\/[\w.-]+$/.test(repo ?? "") || !pubkey) {
  console.error("uso: node scripts/release-config.mjs OWNER/REPO <pubkey>");
  process.exit(1);
}
console.log(
  JSON.stringify({
    plugins: {
      updater: {
        pubkey,
        endpoints: [`https://github.com/${repo}/releases/latest/download/latest.json`],
      },
    },
  }),
);
