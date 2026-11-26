# Wax

<img src="assets/brand/wax-icon-1024.png" width="128" alt="Wax app icon: a red wax seal with a w pressed into it">

Mail, fast. Extend it with plugins.

Wax is a keyboard-first mail app for macOS, written in Rust, drawn on the GPU
with [GPUI](https://gpui.rs) (Zed's UI framework) and backed by a local SQLite
cache. The core reads, writes and files your mail; everything else is a
sandboxed WebAssembly plugin you choose.

**Website:** [tekanic.github.io/wax](https://tekanic.github.io/wax/)

> Early release. Gmail and Google Workspace accounts on Apple silicon Macs.
> There's no signed download yet: build it from source (below).

## Features

**Reading**
- Every header cached on your Mac (SQLite): folders open at once, mail reads
  offline, and search covers mail you haven't opened, merged with Gmail's own
  search across All Mail.
- Conversations: one row per thread, newest first, your replies included;
  archive, delete, read and labels act on the whole thread. Or one row per
  message, if you prefer.
- All Inboxes and All Sent across accounts; search in Sent matches who you
  wrote to, the subject and the text.
- HTML mail in a sandboxed WebKit view: no scripts, remote images and tracking
  pixels blocked until you load them, links open in your browser.
- New mail checked every 30 seconds, with a Dock badge and notifications.

**Writing**
- Messages written in Markdown, sent as clean HTML plus the plain-text source;
  tables pasted from Excel, Numbers or Sheets.
- Reply, reply all and forward from the keys or the reader, with quotes that
  read like the message and threading kept.
- Eight seconds to review and undo every send; drafts autosave locally and save
  to Gmail.
- Signatures designed in the app, From addresses per account, Bcc,
  attachments, address suggestions, and Blocks: reusable Markdown snippets.

**Organizing**
- Every action has a key, and the bottom bar shows the ones that apply.
- Archive, delete, star, Important, labels and bulk select, with undo.
- Sender rules: file a sender's past and future mail in one key, or block them.

**Your Mac, your way**
- Any Zed theme, light and dark, following the Mac's appearance.
- Settings, rules, signatures, keys and themes are plain text in
  `~/.config/wax`, picked up as you save them.
- The layout adapts from a 720px window to an ultrawide; Wax remembers where
  you left it.

## Install

Wax builds from source on an Apple silicon Mac.

```sh
git clone https://github.com/tekanic/wax.git && cd wax
scripts/install.sh
```

`install.sh` makes a release build, signs it with your Apple Development
certificate if you have one, builds the plugins, installs `~/Applications/Wax.app`
and opens it. Run it again to update. You'll need
[Rust](https://rustup.rs) and a Google OAuth client for sign-in; see
[Accounts](#accounts).

## Plugins

New mail goes through one path (`src/plugins.rs`). Sender rules, built in
because the Sweep, the Lobby and block/allow all write them, file mail from
senders you've decided about. What's left goes to the WebAssembly plugins
*before* it's shown or announced, so mail a plugin moves (the Screener's Lobby,
a Filter rule) never flashes into the Inbox or sets off a notification.

The plugins live in
`~/.config/wax/plugins/<id>/plugin.wasm` (interface: `wit/plugin.wit`,
`wax:plugin@0.5.0`). Each call runs on a fresh instance with no files, no
network, 64 MB of memory and a fuel limit: a call that runs out of time is
skipped, and a plugin that keeps doing it is switched off. A new plugin starts off; Settings → Plugins shows
what it asks for and switches it on, which writes its `grants.toml`. Actions
beyond what was granted are refused. Each plugin reads its own `config.toml`.

```sh
scripts/build-plugins.sh            # build plugins/* and install them
```

**Writing one:** [the plugin guide](https://tekanic.github.io/wax/plugins/) ([source](docs/plugins.md)). The interface is MIT licensed, so a plugin can use any license.

**The Sweep** (`plugins/sweep`) is a plugin: it ranks senders and sets the keys
and where each one files a sender's mail (`r = "Reading"   # newsletters` in its
`config.toml`). Wax draws the view and keeps the sender rules, which the
Screener and block/allow use too.

**Snooze & follow-up** (`plugins/snooze`): `z` (or the Snooze button) hides a
message or conversation under a Snoozed label until a time, then puts it back in
the Inbox unread; `w` on a message you sent brings it back if nobody has replied
by then. Pick a time or type one: `fri 9am`, `12/3 5pm`, `in 2 days`,
`tomorrow`. Reminders live in the local cache, survive restarts and are checked
every 30 seconds; snoozing again replaces the time, "Back to the Inbox now"
unsnoozes. Its `config.toml` sets the morning and evening hours.

**Phishing warnings** (`plugins/phishing`): new mail from someone you don't know
gets a **Suspicious** label when it adds up to phishing, and the status bar says
why: a display name claiming a known brand (PayPal, Microsoft, banks, carriers…)
from a domain that isn't theirs, a lookalike of a brand's or a correspondent's
domain (paypa1.com, ernco.com), a brand's name inside a stranger's domain, an
address shown in the name that isn't the sender's, a punycode domain, or a
contact's name from a new domain plus pressure wording ("quick favor", "gift
card", "bank details"). With `read-bodies` it also reads the message: Gmail's
DMARC/SPF/DKIM verdict, a Reply-To pointing elsewhere, links that show a brand's
or contact's site but go somewhere else, links to lookalike domains or bare IP
addresses, and program or web-page attachments. Its `config.toml` sets the label,
trusted domains and extra brands to protect. On 1,684 past messages from
one-time senders it flagged 2, both reasonable; on 315 recent messages it flagged
none of the Inbox and 32 of 135 in Spam (mostly forged senders).
When it's switched on it sweeps the mail you already have (outside Spam and
Trash), and it checks a message again when you open it or move it out of Spam,
so a suspect message carries a ⚠ banner with the reasons wherever you read it.

**Filter rules** (`plugins/filter-rules`) acts on new mail, one rule per line:

```
subject contains "invoice" => label "Receipts"
from ends-with "@github.com" => move "GitHub"
```

## Your files

Everything you set up is plain text in `~/.config/wax/` (like Zed's
`~/.config/zed`), so it can be backed up, kept in git or edited by hand. Wax
picks up a saved change within two seconds; Settings → General → Show folder
opens it.

```
settings.toml                       font, list while writing, theme
themes/<name>.json                  Zed theme files (any number of themes each)
keymap.toml                         every shortcut; defaults listed, commented out
accounts/<email>/account.toml       sender name, sign-in, default signatures
accounts/<email>/rules.toml         that account's sender rules: "address" = "keep" or a label
signatures/<name>/signature.toml    fields, layout, font, links, its account (+ logo.png, logo2.png)
signatures/.deleted/                deleted signatures, kept just in case
identities/<email>.toml             extra From addresses (and the account they send through)
blocks/<name>.md                    blocks for the composer ({{cursor}} = start here)
```

`~/Library/Application Support/gpui-mail/` keeps what isn't a setting:
`cache.sqlite` (only what can be fetched from Gmail again, plus the unsent-message
autosave), `errors.log` (Settings → Problems) and `google-client.json`, your Google
OAuth client. That one is a secret, so it stays out of `~/.config`. Settings from
older versions move to `~/.config/wax` on first start.

`gpui-mail --import-signature <file.html> <name>` brings in an HTML signature
(e.g. from Thunderbird's `sig_file`), logos and links included.

## Themes

Wax uses Zed's theme format, so any Zed theme works unchanged. Built in: Wax
Dark and Wax Light, plus Zed's One, Ayu and Gruvbox families (MIT licensed;
see `assets/themes/*/LICENSE`). Settings → General picks a light theme, a dark
theme, and whether to match the Mac, use light or use dark. Drop more themes
into `~/.config/wax/themes/` (themes in `~/.config/zed/themes/` show up too).
Saving a theme file restyles Wax within two seconds, so you can tune one
while you look at it.

Wax reads these Zed colors and works out anything a theme leaves out:

| Zed key | In Wax |
|---|---|
| `editor.background` | Reading pane, compose |
| `panel.background` | Sidebar |
| `surface.background` | Message list |
| `elevated_surface.background` | Menus and pickers |
| `border.variant`, `border` | Dividers; inputs and menu edges |
| `text`, `text.muted`, `text.placeholder` | Text, secondary text, timestamps and hints |
| `text.accent` | Accent: unread dots, primary buttons, links |
| `ghost_element.hover`, `ghost_element.selected` | List hover and selection |
| `element.background`, `element.hover`, `element.active` | Search field, buttons, pressed buttons |
| `error`, `warning` | Errors, delete confirmations, plugin warnings |

Messages' HTML and signature previews keep a white page, since that's how
they'll look to the people who get them.

## Keyboard

Shortcuts are defined once in `src/keys.rs`; the same table drives the key
bindings, the hint bar at the bottom of the window and the `g` menu.
`~/.config/wax/keymap.toml` changes any of them: a section per place
(`[list]`, `[writing]`, `[global]`…), lines like `"shift-e" = "archive"`, and
`"none"` to switch a key off. Mistakes show in the status bar and Settings → Problems.

| Keys | Where | Does |
| --- | --- | --- |
| `j` / `k`, `↓` / `↑` | list | next / previous message |
| `u`, `⇧⌘U` | list | toggle read / unread |
| `/`, `⌘F` | list, anywhere | search |
| `g` then `i` `s` `t` `d` `a` | list | go to Inbox, Starred, Sent, Drafts, All Mail |
| `⏎`, `↓` | search box | jump into the results |
| `esc` | search box, list | clear the search |
| `c` / `r` / `a` / `f` | list | compose / reply / reply all / forward |
| `⌘⏎` | writing | send (8 s to undo with `⌘Z`) |
| `⌘S` | writing | save draft to Gmail, keep writing |
| `esc` | writing | save draft to Gmail and close |
| `⇧⌘⌫` | writing | discard |
| `⌘B` `⌘I` `⌘⇧X` | writing | bold / italic / strikethrough (again to remove) |
| `⌘K` | writing | link (uses a copied URL) |
| `⌘E` | writing | inline code, or a code block for several lines |
| `⌘⇧8` `⌘⇧7` `⌘'` | writing | bulleted list / numbered list / quote |
| `⌘⇧P` | writing | preview the message as it will arrive |
| `⌘⇧A` | writing | attach files |
| `⌘⇧B` | writing | Bcc on / off |
| `⏎` | Drafts folder | edit the draft |
| `x` / `⌘A` | list | mark one / mark all loaded (`esc` clears) |
| `e` | list | archive marked (or current); in Trash, Spam and Screened Out: back to the Inbox |
| `s` / `i` | list | star / mark important (again to undo) |
| `b` | list | block the sender (Screened Out); in Screened Out: allow them back |
| `⇧R` / `⇧C` | list | file everything from the sender in Reading / Records |
| right-click | list | menu of everything above, for the message or the selection |
| two-finger swipe | list | right: read / unread; left: archive (or the folder's `e`) |
| `⌫`, `#` | list | delete marked (or current) to Trash |
| `⌘Z` | list | undo the last archive, delete or sender decision (8 s) |
| `g w` | list | open the Sweep |
| `r` / `c` / `x` / `i` | Sweep | Reading / Records / screen out / keep (set in the Sweep plugin's `config.toml`) |
| `l` | list | label the message(s); ⇧⏎ in the picker also archives |
| `⌘/` | anywhere | hide / show the hint bar |

Single-letter keys only act when the search box isn't focused, so they can be
typed into a search.

## Labels

Press **`l`** on a message (or on marked messages, or pick "Label…" from the
right-click menu) to open the label picker: type to filter, the message's current
labels are ticked, `⏎` adds or removes the highlighted label, typing a new name
offers to create it, and `⇧⏎` labels and archives. In All Inboxes it uses the
message's own account's labels.

**Settings → Labels** lists each account's Gmail labels: create (use `/` to nest),
rename (nested labels and sender rules follow) or delete (asks twice; the messages
stay in All Mail, only the label goes). The Sweep's Reading, Records and Screened
Out are managed from the Sweep.

## Problems

Status messages fade from the bottom bar (errors after 15 s). Errors, failed
new-mail checks and sync stoppages are kept in `errors.log` in the data folder and
shown under **Settings → Problems**, newest first, filterable by account, with
Copy, Show file and Clear. A red **⚠ n** chip in the bottom bar counts the ones
you haven't looked at; click it to open Problems. The app keeps at most **4 connections per account** (Gmail allows about 15 per
account across all your mail apps): normally 2 for the account on screen (3 while
older mail is still syncing) and 1 for each other account, which its new-mail check
shares. Settings → Accounts shows how many are open. If the app can't connect (for
instance Gmail refusing logins while too many connections are open across your
mail apps), it tries again after 15 s, 30 s, 1 min and then every 2 min.

## Accounts

Add accounts in **Settings → Accounts**. Gmail and Google Workspace accounts use
**Sign in with Google**: your browser opens Google's sign-in page and the app never
sees your password. Tokens are kept in the macOS Keychain (service
`gpui-mail-google`). Each account has its own sender name, default signatures,
From addresses and sender rules, and its own new-mail watcher.

The sidebar keeps **All Inboxes**, **All Sent** and your accounts pinned at the
top (visible however short the window), with the folders of the account on
screen below. All Inboxes merges every account's Inbox, newest first, each row
tagged with its account; All Sent does the same for Sent, and a search there
covers every account's sent mail; reading, replying, archiving, starring and sender decisions there act on
the message's own account.

Google sign-in needs an OAuth client:

- **Release builds** can carry one, set at build time with
  `GPUI_MAIL_GOOGLE_CLIENT_ID` and `GPUI_MAIL_GOOGLE_CLIENT_SECRET`. Google limits an
  unverified app with Gmail access to 100 users, so a public release needs Google's
  verification for the `https://mail.google.com/` scope.
- **Without one**, each user brings a client from their own Google Cloud project
  (free, about 10 minutes; the Accounts screen lists the steps): turn on the Gmail
  API; set up Google Auth Platform with Audience *External* and the scope
  `https://mail.google.com/`; create a *Desktop app* client and choose its
  downloaded JSON in the app; then **Publish** the app, since in *Testing* Google
  signs everyone out every 7 days. Each account then sees a one-time "Google hasn't
  verified this app" screen (Advanced → Go to …).

Test a login without the UI: `cargo run -- --check` (first account).

## Roadmap

**Getting it into people's hands**
- [ ] Google verification for Gmail access, so nobody has to bring their own OAuth client
- [ ] Signed, notarized downloads with automatic updates
- [ ] Push notifications for new mail

**Plugins**
- [x] Plugin host: sandboxed WebAssembly with permissions, fuel and memory limits
- [x] The Sweep, Screener, Snooze & follow-up, Phishing warnings, Filter rules
- [ ] Spam filter that learns from what you move to Spam or Screened Out
- [ ] Spelling and grammar while you write
- [ ] MCP server, so an AI assistant can search and draft with the access you grant
- [ ] AI summaries and a morning brief, with your own API key, run on your Mac
- [ ] A place to find and install plugins, including paid ones

**Mail**
- [ ] Calendar invitations: see and answer them in the message
- [ ] Contacts
- [ ] Send later
- [ ] Other providers: Outlook, iCloud and plain IMAP
- [ ] VoiceOver and keyboard accessibility review

Ideas and bug reports are welcome in [Issues](https://github.com/tekanic/wax/issues).

## Development

Requires Rust 1.100+ (pinned to `stable` in `rust-toolchain.toml`). If
Homebrew's Rust shadows rustup's, put rustup's toolchain first:

```sh
T=~/.rustup/toolchains/stable-aarch64-apple-darwin/bin
PATH=$T:$PATH RUSTC=$T/rustc cargo run
```

`cargo test` runs the host's tests, including the built plugins;
`scripts/build-plugins.sh` builds `plugins/*` for `wasm32-wasip2`;
`scripts/bundle.sh debug` makes a debug bundle in `target/debug/Wax.app`;
`cargo run -- --check` tests a login without the UI. The repository and binary
are called `gpui-mail`; the app is Wax.

**The website** lives in `docs/` and is served by GitHub Pages.
`scripts/build-site.sh` (needs pandoc) builds the user guide from this README
and the plugin guide from `docs/plugins.md`. `scripts/screenshots.sh` (needs
`cwebp`) retakes the screenshots in `docs/shots/` from **demo mode**: Wax with
`WAX_DEMO=1` and a throwaway `HOME` filled with made-up mail by
`gpui-mail --make-demo`, which never touches the network or your real mail.
Set `WAX_SHOT_POS` to a spot on a Retina display for sharp captures.

- `src/main.rs`: app shell, panes, key handling; `src/keys.rs`: the keymap
- `src/mail.rs`: Gmail over IMAP and SMTP with OAuth; `src/store.rs`: the SQLite cache
- `src/compose.rs`, `src/compose_view.rs`: writing, Markdown, sending
- `src/plugins.rs`, `src/plugin_checks.rs`: new mail through rules and plugins, existing-mail checks
- `src/wasm_host.rs`: the sandboxed plugin host (wasmtime); `wit/plugin.wit`: the plugin interface
- `src/theme.rs`: Zed theme loading; `src/html_view.rs`: the HTML mail view
- `plugins/`: the bundled plugins, one crate each (`test-*` are for the host's tests)
- `docs/`: the website, the guides and their screenshots; `src/demo.rs`: demo mode

## License

Wax is free software under the [GNU General Public License v3.0](LICENSE) or
any later version: use it, change it and share it, and share your changes
under the same terms.

The plugin interface (`wit/`) and the plugins in `plugins/` are MIT licensed
([wit/LICENSE-MIT](wit/LICENSE-MIT), [plugins/LICENSE-MIT](plugins/LICENSE-MIT)),
so plugins can be written and shared under any license. Bundled themes and
icons keep their own MIT and ISC licenses (`assets/themes/*/LICENSE`,
`assets/icons/LICENSE-LUCIDE`).

Copyright © 2026 Tekanics LLC.

