# Kura (蔵 — the storehouse that guards what matters)

**A local EVM wallet for AI agents.**
On a Mac, when an AI like Claude pays for something online (x402), Kura is the wallet where a human approves it with a password.

- Not a wallet for people (MetaMask and friends) — **a wallet for AI agents**
- Not a cloud SaaS — it runs **locally only**. The key never leaves this computer.
- The AI **asks** to pay → a human **approves with a password** → it goes out
- The chain is **Base** (an Ethereum L2, fees near zero). Mainnet by default; you can switch to the practice testnet (Base Sepolia) in settings

한국어 문서: **[README.md](README.md)**

---

## At a glance

```
[ AI app (Claude Code / desktop) ]
        │  asks to pay, over MCP
        ▼
[ Kura desktop app ]  ← approval window → the human types their password
        │  signs (the key stays inside this app)
        ▼
[ Base chain / x402 facilitator ]  ← the actual payment
```

Your password is typed **in the Kura app alone**. It never enters a chat window, MCP, or a config file.

---

## Install

**Apple Silicon Mac (macOS 11 or newer)**

### (A) Homebrew

```bash
brew install --cask dinggi5/tap/kura
```

### (B) Download the DMG

Grab `Kura_<version>_aarch64.dmg` from [Releases](https://github.com/dinggi5/kura/releases/latest), open it, and drag Kura into your `Applications` folder.

This app holds your keys, so **checking that what you downloaded is really ours** matters — [SECURITY.md](SECURITY.md) has the commands.

### (C) Build from source

You'll need [Rust](https://rustup.rs), [Node.js](https://nodejs.org) (20.19+ or 22.12+ — Vite 7 requires it), and macOS.

```bash
git clone https://github.com/dinggi5/kura.git
cd kura
git checkout v0.2.1   # the same source as the release. Omit it to get the latest development code
npm ci                # installs exactly what package-lock.json says (install may bump versions)

# run it in development mode
npm run tauri dev

# build the app and install it into /Applications
npm run tauri build -- --bundles app --no-sign
ditto src-tauri/target/release/bundle/macos/Kura.app /Applications/Kura.app
open /Applications/Kura.app
```

### Updates

**The app updates itself.**
Open **Settings → About**, read the version and what changed, and press **Install now and restart**.

- Only the check is automatic — **installing is always your press**.
- A downloaded update installs only after its signature checks out (signature verification cannot be turned off). The "Updates" section of [SECURITY.md](SECURITY.md) has the details.
- If you'd rather it never phoned home, turn off **Settings → About → Check at startup**.

### Uninstalling

⚠️ **Removing the app does not remove your wallet.** Your key, settings, and history live in `~/.jigap`, outside the app, so deleting the app leaves them untouched — deliberately, so nobody loses funds by accident (`brew uninstall --zap` won't remove them either).

```bash
# remove just the app (the wallet stays — reinstall and you pick up where you left off)
brew uninstall --cask dinggi5/tap/kura   # or drag /Applications/Kura.app to the Trash
```

To remove **the wallet as well** from this Mac, follow the order:

1. **Make sure you have your 12 recovery words first.** The key button in the app's header shows them.
2. Move any remaining balance to another wallet.
3. **Quit Kura completely** (right-click the menu-bar icon → Quit). While it runs it writes state into `~/.jigap` every few seconds, so a deleted folder reappears immediately.
4. Quit any AI tool you connected Kura to (Claude Code and so on).
5. If you enabled start-at-login, turn it off, or `rm ~/Library/LaunchAgents/Kura.plist` (dragging the app to the Trash leaves this file behind; `brew uninstall --cask` takes it with it).
6. Then `rm -rf ~/.jigap`

If you run that last `rm -rf ~/.jigap` without your 12 words, **nobody can recover the funds in that wallet.**

---

## First run

> **Kura lives in your menu bar.** Launch it and a keyhole icon (◉) appears at the top of your screen; the window drops down only when you click it. Click elsewhere and it closes, while the app stays in the background waiting for AI payment requests. To quit for good, **right-click the icon → Quit**. (On the very first run, with no wallet yet, the window opens on its own.)

1. **Create a wallet** — choose the password (8 characters or more) you'll type for every payment. Your key is stored encrypted with it (`~/.jigap/wallet.enc`).
2. **Back up your words** — twelve words appear. They are **the real proof the funds are yours**, so write them on paper or keep them in a password manager. Even if you forget your password they bring the funds back: quit the app, **check that you have the twelve words**, delete `~/.jigap/wallet.enc`, relaunch, and use **Import** on the first screen (any standard BIP-39 wallet works too). Delete that file without the twelve words and the funds are gone for good.
3. **Welcome tour** — it walks through topping up, connecting an AI, and the safety rails. The **ⓘ Help** button in the header shows it again any time.

---

## Adding money (USDC)

To pay for anything, the wallet needs **USDC** — a digital dollar.

- Press **Receive** in the app to see your address and its QR code.
- Sending from an exchange or another wallet? You must pick the **Base network**. A different network loses the funds.
- **Mainnet (the default) is real money** — top up only what the agent needs to spend. To practice first, switch to the **testnet** in settings and use the Faucet buttons on the Receive screen for free test coins.
- **ETH is optional** — with x402 the facilitator covers the fee, so a zero ETH balance is fine. Only a direct transfer from the app's **Send** screen needs a little ETH for gas (Base fees run around a cent). Topping up USDC alone is enough.

---

## Connecting an AI (Claude)

Register the Kura server in your AI app's **MCP settings** and they're connected — the main screen then shows a **"Claude connected"** badge. Since 0.1.2 the **MCP server ships inside the app**, so there's no repo to clone and no Rust to install.

### The easy way — the app's "Connect an AI" screen

Tap the **"No AI connected" badge** at the top of the main screen.

- **Claude desktop** — press "Connect" and Claude opens its extension installer. Press Install and you're done.
- **Claude Code** — one button registers it (the app runs `claude mcp add` for you). From the next `claude` run on, it works in any folder.

Below is the same thing done by hand.

### Claude desktop — one extension file

Download `kura-<version>.mcpb` from the [releases page](https://github.com/dinggi5/kura/releases/latest) and **double-click it**; Claude desktop asks to install it. (Or Claude Settings → Extensions → pick the file.)

The extension contains no executable — only a launcher that **verifies the signature** of the notarized MCP server inside the installed Kura.app and runs that. This is why Kura itself must be installed first.

### Claude Code and other MCP apps

Register the binary inside the app by absolute path:

```bash
claude mcp add --scope user kura -- /Applications/Kura.app/Contents/MacOS/kura-mcp
```

(`--scope user` makes it available everywhere instead of "this folder only". Drop it if you want it in one project.)

Other MCP apps take a config like this:

```json
{
  "mcpServers": {
    "kura": {
      "command": "/Applications/Kura.app/Contents/MacOS/kura-mcp"
    }
  }
}
```

### Working from source

The repo root already has an [`.mcp.json`](.mcp.json). Run `claude` in this folder and it picks it up (it asks whether to use the server the first time — **approve**). That path rebuilds with `cargo run` every time, which is what you want while developing; if you're just using the app, register the app path above.

Tools the AI gets: `get_wallet_status` · `get_balances` · `get_history` (read only) · `request_payment` (asks to pay → approval window in the app) · `x402_fetch` (calls a URL behind an x402 paywall).

> Changing MCP tools means **restarting the AI app** — the server loads once per session.

---

## How a payment goes

1. The AI asks to pay.
2. The Kura window appears and shows **how much, to whom** (it comes forward even when hidden).
3. You type your password and approve.
4. The payment goes out and the result goes back to the AI. **No approval within 5 minutes and it's rejected automatically.**

---

## Security model

- **Password approval** — by default every payment waits for your password (autopay, below, is the one exception). The key is stored encrypted (Argon2id + AES-256-GCM), decrypted only to pay, and wiped right after (with an autopay session on, it stays in memory only while unlocked).
- **Limits** — you set how much can go out per payment and per day (5 and 20 USDC by default). Anything over is blocked.
- **Emergency lock** — the shield button in the header blocks every payment at once.
- **Trusted addresses · autopay** — approving without a password needs all three: *an unlocked session, an amount under the small limit, and an address you've approved before*. Everything else asks.
- **History** — every payment sent, attempt blocked, and signature made is recorded.
- **Local first** — the key (`~/.jigap/`) lives outside the repo and is never committed.
- **No analytics, no font CDN** — the app reaches the internet in exactly three places: ① the RPC server used to read balances and send payments, ② the x402 URL the AI asked for, and ③ the update check on GitHub. Nothing about your usage is sent anywhere, and the typeface ships inside the app, so the UI renders fine offline.
- **The update check can be turned off** — on launch the app asks GitHub whether a newer version exists (your IP and current version show up there). Turn off **Settings → About → Check at startup** and that request stops too. Either way, **installing is always your press** — nothing changes behind your back.

---

## For developers

The `kura` CLI drives the same wallet (it shares the core with the MCP server). It's already inside the installed app, so one symlink puts it on your PATH:

```bash
sudo mkdir -p /usr/local/bin
sudo ln -sf /Applications/Kura.app/Contents/MacOS/kura-cli /usr/local/bin/kura
kura status
```

From source:

```bash
cargo run --manifest-path ./kura-mcp/Cargo.toml --bin kura -- status
cargo run --manifest-path ./kura-mcp/Cargo.toml --bin kura -- balance
cargo run --manifest-path ./kura-mcp/Cargo.toml --bin kura -- history --limit 10
cargo run --manifest-path ./kura-mcp/Cargo.toml --bin kura -- pay <address> <amount> --token usdc
cargo run --manifest-path ./kura-mcp/Cargo.toml --bin kura -- fetch <URL>
```

`pay` and `fetch` **never take your password on the CLI** — the Kura app has to be running, and a human types the password in the approval window.

The CLI follows the app's language. `KURA_LANG=en` (or `ko`) overrides it for one run.

Tests:

```bash
(cd src-tauri && cargo test)        # backend (wallet, crypto, limits, transfers)
(cd kura-mcp && cargo test)         # MCP/CLI adapters
npx tsc --noEmit && npx vite build  # frontend
```

Building a release (a signed, notarized DMG) is written up in **[docs/RELEASE.md](docs/RELEASE.md)** — in Korean. It needs an Apple Developer account and a one-time setup; after that it's one line:

```bash
./scripts/release.sh
```

### Stack

| | |
|---|---|
| Desktop | Tauri (Rust + web frontend) |
| Frontend | React + Tailwind CSS, Framer Motion, Lucide, Pretendard |
| Chain | Base / alloy-rs |
| Payments | x402 (EIP-3009 off-chain signatures) |
| AI connection | MCP server (rmcp) |

Shape: `[Rust core (src-tauri)] ← MCP / CLI adapters (kura-mcp)`. Only the GUI process can reach the key to sign — that's the last line of defense.

### Languages

The app speaks **Korean and English**, chosen in **Settings → App → Language**; with nothing chosen it follows your macOS language. Picking one reopens the window in that language.

The MCP server is English-only on purpose — an LLM reads it, and tool descriptions are compile-time constants. Your agent still answers you in your own language.

---

## License · status

**[MIT](LICENSE)** — this is code that handles keys and money, so you should be able to read it, verify it, and change it.

Found a vulnerability? Please use the private path in **[SECURITY.md](SECURITY.md)** before opening an issue. The same document lists **what Kura does not protect you from**.

The bundled [Pretendard](https://github.com/orioncactus/pretendard) typeface is under the **SIL Open Font License 1.1** ([full text](public/fonts/LICENSE-Pretendard.txt)).

Kura's first user is its author — a wallet for people on an Apple Silicon Mac who let a local LLM or Claude spend on their behalf. It's early software: put no more on mainnet than you can afford to lose, and if any of this is new to you, switch to the testnet in settings and practice there first.
