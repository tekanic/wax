# Wax

<img src="assets/brand/wax-icon-1024.png" width="128" alt="Wax app icon: a red wax seal with a w pressed into it">

Mail for the Mac. The rest is plugins.

Wax is a bare-bones, plugin-extensible email client for macOS, built in Rust on
[GPUI](https://gpui.rs) (Zed's UI framework) via [gpui-kit](https://gpui-kit.com).

The repository and binary are still called `gpui-mail`; the app shows as Wax.
Install it with `scripts/install.sh`: a release build, signed with your Apple Development
certificate if you have one, installed as `~/Applications/Wax.app` and opened. Run it again
to update. `scripts/bundle.sh debug` makes a debug bundle in `target/debug/Wax.app`.

The core only does mail. Everything else (rules, snooze, AI actions,
integrations) will ship as sandboxed WebAssembly plugins.

## Status: Phase 0 spike

- [x] Native window, three panes: mailboxes | thread list | reader
- [x] Virtualized thread list with 10,000 generated messages
- [x] Keyboard navigation: `j`/`down`, `k`/`up`, `cmd-q`
- [x] Read-only IMAP via Pimalaya io-email, verified against Gmail (login ~0.3 s)
- [x] HTML email reduced to readable text with html2text (real rendering still open)
- [x] Draft plugin interface: `wit/plugin.wit` (validates with `wasm-tools component wit wit/`)
- [x] Verify against Gmail with `--check`
- [x] Plugin host prototype (wasmtime) + one sample plugin (Filter rules)

## Phase 1: read-only client (in progress)

- [x] Local cache (SQLite at `~/Library/Application Support/gpui-mail/cache.sqlite`):
      folders, envelopes, previews and fetched bodies; opens instantly, works offline
- [x] Refresh: server window upserted into the cache; archived/deleted rows dropped
- [x] Search box (`cmd-f` or `/`) over all cached mail, de-duplicated across Gmail folders
      (`cargo run -- --search <terms>` from the shell)
- [x] Server-side search (Gmail `X-GM-RAW`, all of All Mail), merged with local hits.
      In Sent (an account's, or All Sent) search stays in sent mail and matches
      recipients too: who you wrote to, the subject or the text.
- [x] Conversations: reading a message shows every message in its Gmail thread
      (X-GM-THRID), your replies included, above it; click one to read it in place.
      The list shows one row per conversation (the newest message, with the
      thread's size); archive, delete, read/unread and labels act on the whole
      thread, star on its newest message. Settings → General → Message list
      switches back to one row per message (`conversations` in settings.toml).
      Sent rows show who the message went to.
- [x] Read state syncs to Gmail (open = read, `u` toggles), rolled back on failure
- [x] Older mail: scroll loads the next 200 (cache first, then Gmail); background
      header sync of All Mail (~34k messages in ~75 s, resumable, UIDVALIDITY-aware)
- [x] HTML email in a native WebKit view (wry): scripts and remote loads blocked by CSP,
      per-message "Load images", links open in the default browser
- [x] Images inside messages: `cid:` images embedded in place, attached photos in a
      gallery under the message (also for plain-text mail); no network access

## Phase 2: writing (in progress)

- [x] Compose (`c`), reply (`r`), reply all (`a`), forward (`f`) in the reader pane;
      replies quote the original and keep threading (In-Reply-To / References)
- [x] Send with `⌘⏎` through smtp.gmail.com (same app password). The message stays
      on screen read-only for a last review, under a countdown banner (8 lamps going
      out, pulsing number, then Sending… / Sent once Gmail accepts it) for 8 s: `esc`/`⌘Z` cancels, `⏎` sends now. Failures reopen the message
- [x] Drafts: autosaved locally while typing (restored after a crash or quit);
      `⌘S` or Save saves to Gmail's Drafts and keeps writing (saving again replaces
      it); `esc` saves and closes; `⏎` or Edit on a message in Drafts reopens it;
      `⇧⌘⌫` discards
- [x] Address suggestions in To/Cc from people you've written to (Sent folder,
      ranked first) and frequent senders; `↑↓` choose, `⏎`/`tab` add, `esc` dismiss
- [x] Markdown: messages are written in Markdown and sent as text/plain (the
      source) + text/html (GitHub-flavored, inline-styled, line breaks kept, bare
      URLs linked). Toolbar and shortcuts for bold, italic, strikethrough, links,
      code, lists and quotes; `⌘⇧P` previews it as it will arrive, and the send
      countdown shows the rendered message
- [x] Actions in every folder: star (`s`), Important (`i`), block or allow a sender
      (`b`), file a sender (`⇧R`/`⇧C`), right-click menu, swipes; `e` means the
      folder's natural move (archive, restore, not spam)
- [x] Settings (`⌘,` or the button beside the Sweep): writing font (monospaced by
      default, switchable per message with `⌘⇧M`), hide the message list while
      writing, default signatures for new messages and for replies
- [x] Signatures: several per account, designed in Settings → Signatures (layout,
      details, colour, logo) with a live preview of how they arrive. Kept apart
      from the text and switched in one click or with `⌘⇧S`; placed above any
      quote; the logo is embedded (cid), so it shows without "load images"
- [x] Tables: `⌘V` turns a copied Excel / Numbers / Sheets range into a table,
      `tab` / `⇧tab` move between cells and line the columns up (a new row after
      the last cell), the Table button (`⌥⌘T`) inserts one; number columns
      right-align
- [x] Blocks: while writing, the message list's column lists blocks (Markdown
      files in `~/.config/wax/blocks/`, `{{cursor}}`
      marks where typing resumes). Drag one into the message or click to add it
      at the cursor
- [x] From addresses (identities, as in Thunderbird): Settings → From addresses adds
      extra addresses with their own signatures; a From picker in the composer; replies
      come from the address the message was sent to. Gmail needs each one under
      Settings → Accounts → "Send mail as"
- [x] New mail on its own: a watcher on its own connection checks the Inbox every
      30 s (IMAP STATUS), fetches only new headers, runs sender rules, adds the rest
      to the list (selection kept), sets the Dock badge to the Inbox's unread count
      and posts a notification; the open folder is re-read every 5 min or when the
      Inbox shrinks (mail archived elsewhere)
- [x] Attachments: received files are listed under the sender; click to open,
      the arrow saves to Downloads, "Save all" for several. Saved files carry the
      macOS quarantine mark, and programs or scripts are shown in Finder rather
      than opened. To send: Attach (`⌘⇧A`) or drag files in from Finder;
      forwards and reopened drafts keep their files; sends over Gmail's 25 MB
      limit are stopped before they go.
- [x] Bcc (`⌘⇧B` or "Bcc" beside the Cc line opens it; × or `⌘⇧B` on an empty line closes it, and × clears what was typed): blind recipients get the
      message through the mail server's envelope only; the sent message never
      carries a Bcc line. Drafts keep it so they reopen with it.
- [x] Bulk select (`x`, `⌘A`, `esc` clears), archive (`e`), delete (`⌫`/`#`): rows
      leave the list at once, Gmail is changed after an 8 s undo window (`⌘Z`);
      archive = move to All Mail (drops Inbox/label), delete = move to Trash
- [x] The Sweep (`g w` or the sidebar): senders ranked by volume from the cached
      headers, with % never opened, latest date and recent subjects. One key files
      all of a sender's mail, past and future: `r` Reading, `c` Records, `x` Screened
      Out (Gmail labels; mail leaves the Inbox), `i` keep. 8 s undo window (`⌘Z`).
      Rules file new Inbox mail from those senders on every refresh.
- [x] Screener (a plugin, `plugins/screener`; list the accounts to screen in its
      `config.toml` as `screen = "you@example.com"`): a first email from someone new
      waits in the **Lobby** label instead of the Inbox. In the Lobby, `e` lets the
      sender in (their mail goes to the Inbox and they stay there from now on) and
      `b` screens them out; ⇧R / ⇧C file them in Reading or Records. Known senders
      come straight through: anyone you've written to, anyone you've had other mail
      from, colleagues on your work domain, anyone with a sender rule and your own
      addresses. Only mail that arrives after you add an account is screened, and
      never mail more than 3 days old.

### Plugins

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

**Writing one:** [docs/plugins.md](docs/plugins.md).

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

Bodies are fetched with `BODY.PEEK`. The only change written to the server is
read state: opening a message marks it read; `u` toggles read/unread.

## Build and run

Requires Rust 1.100+ (pinned to `stable` in `rust-toolchain.toml`).
If Homebrew's Rust shadows rustup's, run with the rustup toolchain explicitly:

```sh
T=~/.rustup/toolchains/stable-aarch64-apple-darwin/bin
PATH=$T:$PATH RUSTC=$T/rustc cargo run
```

## Layout

- `src/main.rs` — app shell, panes, key bindings
- `src/mail.rs` — IMAP session (io-email), Keychain lookup, `--check`
- `wit/plugin.wit` — plugin interface (actions, commands, panels, permissions)
- `src/wasm_host.rs` — sandboxed plugin host (wasmtime); `src/plugins_pane.rs` — Settings → Plugins
- `plugins/` — plugins, each its own crate built for `wasm32-wasip2` (`test-*` are for the host's tests)
- `src/model.rs` — placeholder envelope model and fake data (replaced by the
  SQLite store in Phase 1)
