# Writing a Wax plugin

Wax keeps its core small. Features like the Sweep and Filter rules are plugins:
WebAssembly components that Wax runs in a sandbox. This guide takes you from an
empty folder to a working plugin, then covers the whole interface.

- Interface: [`wit/plugin.wit`](../wit/plugin.wit), package `wax:plugin@0.3.0`
- Host: [`src/wasm_host.rs`](../src/wasm_host.rs)
- Examples: [`plugins/sweep`](../plugins/sweep) (a sender view),
  [`plugins/filter-rules`](../plugins/filter-rules) and [`plugins/screener`](../plugins/screener) (act on new mail),
  [`plugins/snooze`](../plugins/snooze) (commands with a menu of choices)

## How plugins work

- **A plugin is a component.** It's built for `wasm32-wasip2` and implements
  the `plugin` world in `wit/plugin.wit`. Rust with `wit-bindgen` is the tested
  path. Any language that produces WASI 0.2 components should work but hasn't
  been tried.
- **Plugins ask; the core acts.** A plugin never changes mail itself. It
  returns `action`s (add a label, archive, file a sender…). Wax checks each one
  against what you allowed, then carries it out.
- **Plugins never draw.** Wax draws everything with its own components. A
  plugin describes what to show (for example a sender view's keys and rows).
- **Every call is sandboxed.** Each call gets a fresh instance with no files,
  no network, no environment variables, at most 64 MB of memory and a fuel
  budget of about 500 million instructions. A plugin that traps, loops or runs
  out of memory is switched off until Wax restarts, and Wax says why.
- **Nothing runs until you say so.** A new plugin shows up in Settings →
  Plugins switched off. Switching it on grants the permissions it asked for.

## Quick start

This plugin stars any new mail whose subject mentions "urgent".

**1. Make the crate.** Put it in `plugins/` so it can find the interface:

```sh
mkdir -p plugins/urgent/src
```

`plugins/urgent/Cargo.toml`:

```toml
[package]
name = "wax-urgent"
version = "0.1.0"
edition = "2021"
publish = false

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "0.63"

[profile.release]
opt-level = "s"
lto = true
strip = true

# Built on its own, not as part of the app.
[workspace]
```

**2. Implement the world.** `plugins/urgent/src/lib.rs`:

```rust
wit_bindgen::generate!({ world: "plugin", path: "../../wit/plugin.wit" });

use wax::plugin::types::Permission;

struct Urgent;

impl Guest for Urgent {
    fn manifest() -> Manifest {
        Manifest {
            id: "urgent".into(),
            name: "Urgent".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            permissions: vec![Permission::ReadEnvelopes, Permission::ModifyMail],
            network_hosts: vec![],
        }
    }

    fn on_message_received(message: Envelope) -> Vec<Action> {
        if message.subject.to_lowercase().contains("urgent") {
            vec![Action::Star(true)]
        } else {
            vec![]
        }
    }

    // The rest of the world. Return "nothing" for the parts you don't use.
    fn commands() -> Vec<Command> { vec![] }
    fn run_command(_id: String, _choice: Option<String>, _message: Option<Envelope>) -> Result<Vec<Action>, String> {
        Err("no commands".into())
    }
    fn panel() -> Option<Panel> { None }
    fn get_sender_view(_account: String) -> Option<SenderView> { None }
    fn on_sender_key(_account: String, _who: Sender, _key: String) -> Result<Vec<Action>, String> {
        Ok(vec![])
    }
}

export!(Urgent);
```

**3. Build and install.**

```sh
scripts/build-plugins.sh urgent
```

This builds `plugins/urgent` for `wasm32-wasip2` and copies it to
`~/.config/wax/plugins/urgent/plugin.wasm`. If the crate has a
`config.example.toml`, the script also copies it to `config.toml` the first
time.

**4. Switch it on.** Within two seconds, Wax notices the new folder. Open
Settings → Plugins, check what it asks for and choose **On**.

## On disk

```
~/.config/wax/plugins/<id>/
  plugin.wasm    the component
  grants.toml    on/off and what you allowed (written by Wax and Settings)
  config.toml    the plugin's own settings, edited by you
  storage.toml   the plugin's key-value store (needs `storage`)
```

`grants.toml` looks like this:

```toml
enabled = true
permissions = "read-envelopes modify-mail"
```

Wax reloads the plugins when any `plugin.wasm` or `grants.toml` changes. A
plugin can only be granted permissions its manifest asks for.

## Life of a call

Wax compiles each plugin once at load. Every call (`manifest`,
`on-message-received`, `get-sender-view` and the rest) then runs on a **new
instance**. Nothing survives between calls: statics and globals start fresh
each time. Keep settings in `config.toml` (read with `config()`) and anything
you learn in storage (`storage-get` and `storage-set`).

| Export | When Wax calls it | Needs |
|---|---|---|
| `manifest` | At load, and whenever the plugins reload | — |
| `on-message-received` | Each new Inbox message that sender rules didn't file, **before** it's shown or announced | `read-envelopes` |
| `get-sender-view` | When you open the Sweep (`g w`); the first running plugin that returns a view wins | `read-envelopes` |
| `on-sender-key` | When you press one of your view's keys on a sender row | `read-envelopes` |
| `commands` | At load, and each time one of your commands' menus opens (so choices can show current times) | — |
| `run-command` | When you press a command's key in the list, click its reader button, or pick from its menu | — |
| `panel` | Not called yet; return `None` | — |

The `envelope` a plugin gets has the sender, subject, date and flags, plus
`to` (To and Cc recipients) and `labels` (Gmail's labels for the message; system
ones start with a backslash, like `\Important`). Message text isn't included;
that will come with `read-bodies`.

A message a plugin moves or archives never appears in the Inbox or as a
notification. The actions themselves then run in the background over IMAP,
labels and flags first, then the move or archive. Wax waits for every plugin
before showing new mail, so keep `on-message-received` quick.

[`plugins/screener`](../plugins/screener) is a complete example: it holds mail
from strangers in the Lobby using `knows-sender`, and remembers when each
account was switched on in storage.

## Permissions

| Permission | Lets the plugin | Status |
|---|---|---|
| `read-envelopes` | Get messages' senders, recipients, subjects, flags; call `senders` | Works |
| `modify-mail` | Return label, move, archive, read, star and file-sender actions | Works |
| `storage` | Use `storage-get` and `storage-set` | Works |
| `read-bodies` | Call `message-text` | Answers "not available yet" |
| `network` | Call `http-get` for hosts in `network-hosts` | Answers "not available yet" |
| `compose` | Return `open-draft` | Not carried out yet |

Ask for the least you need. Settings shows each request in plain words, and an
action you weren't granted is refused and reported.

## Actions

| Action | Effect | Status |
|---|---|---|
| `add-label(name)` | Adds a label, creating it if needed | Works |
| `remove-label(name)` | Removes a label | Works |
| `move-to(name)` | Labels it and takes it out of the Inbox | Works |
| `archive` | Takes it out of the Inbox | Works |
| `mark-read(bool)` | Read or unread | Works |
| `star(bool)` | Star or unstar | Works |
| `notify(text)` | Shows text in the status bar (needs no permission) | Works |
| `file-sender(decision)` | Files all of a sender's mail, past and future (sender views only) | Works |
| `snooze-until(time)` | From a command: out of the Inbox under a Snoozed label until the RFC 3339 time, then back unread | Works |
| `follow-up(time)` | From a command: at the time, back to the Inbox unless someone else wrote in the thread since | Works |
| `open-draft(draft)` | Open a prefilled draft | Not yet |

## Host functions

All live in `wax::plugin::host`.

| Function | Does | Needs |
|---|---|---|
| `log(text)` | Reports a line in the status bar and in Settings → Problems (at most 20 per call) | — |
| `config()` | Your `config.toml` as text, or `""` | — |
| `utc-offset()` | Seconds east of UTC on this Mac now, for local times | — |
| `senders(account, limit)` | Senders by volume from the local cache, with their rule | `read-envelopes` |
| `knows-sender(message)` | Whether you know its sender: your own address, a colleague at your work domain, someone you've written to, someone with a sender rule, or other mail from them (not the Lobby, Spam, Trash or this batch) | `read-envelopes` |
| `storage-get(key)`, `storage-set(key, value)` | Your key-value store | `storage` |
| `message-text(...)` | Message text | Not yet |
| `http-get(url)` | HTTPS GET | Not yet |

`config.toml` is yours to define; Wax doesn't parse it. The built-in plugins use
one setting per line with `#` comments. That's a good habit: people edit these
files by hand.

## Commands

A command acts on the selected message. Wax offers it as a key in the message
list and, with `in-message-toolbar`, a button in the reader.

```rust
Command {
    id: "snooze".into(),
    title: "Snooze".into(),
    keybinding: Some("z".into()),
    in_message_toolbar: true,
    scope: CommandScope::Any,
    choices: vec![CommandChoice { id: "tomorrow".into(), title: "Tomorrow".into(), detail: "Thu 8 AM".into() }],
    text_hint: Some("or type a time: fri 9am".into()),
}
```

- **Keys** are one letter, optionally `shift-`. Pick one Wax doesn't use (`z`
  and `w` are the Snooze plugin's). Keys Wax already binds win.
- **Scope** limits it to messages you sent (`sent`), received (`received`) or
  any.
- **Choices** and **text-hint**: when either is set, running the command opens
  a menu. Picking a choice calls `run-command` with its id; typing and pressing
  Enter sends `"text:<what was typed>"`. With neither, it runs straight away.
- `run-command` gets the message being read. Its actions apply to the whole
  conversation when you ran it from a list row, or just that message when you
  picked one in the conversation strip.

## Sender views

A sender view is a keyboard-driven list of senders, like the Sweep. The plugin
supplies the rows and the keys. Wax draws the list, moves with `j`/`k` and the
arrows, and closes with `esc`. Wax also holds each decision for the undo
window (`⌘Z`), then writes the sender rule and moves the mail.

```rust
fn get_sender_view(account: String) -> Option<SenderView> {
    let rows = wax::plugin::host::senders(&account, 500).unwrap_or_default();
    Some(SenderView {
        title: "Sweep".into(),
        keys: vec![ViewKey { key: "r".into(), label: "Reading".into(), about: "newsletters".into(), color: 0x4f8ff7 }],
        rows,
    })
}

fn on_sender_key(_account: String, who: Sender, key: String) -> Result<Vec<Action>, String> {
    if key != "r" { return Ok(vec![]); }
    Ok(vec![Action::FileSender(SenderDecision {
        email: who.email,
        name: who.name,
        count: who.count,
        destination: Some("Reading".into()), // None keeps them in the Inbox
    })])
}
```

- **Keys** are single characters pressed without modifiers. `j`, `k` and `esc`
  are taken by the view.
- **Rows** come back to you in `on-sender-key` as the `who` argument, with
  their current `rule` so you can skip decisions already made.
- **Colours:** a row whose rule matches a key's `label` is drawn in that key's
  colour.
- Only `file-sender` and `notify` work from a sender view.

See [`plugins/sweep`](../plugins/sweep) for the full version, which reads its
keys from `config.toml`.

## Testing

**Plain logic** (parsing your config, matching rules) can be unit tested with
`cargo test` in the plugin's folder. This works as long as the code under test
doesn't call host functions. `plugins/sweep` does this.

**The whole plugin** can be tested against the real host. Build it, then follow
the tests at the bottom of `src/wasm_host.rs`: copy the `.wasm` into a scratch
folder with a `grants.toml`, load it with `Host::load_from`, and call
`arrived`, `sender_view` or `sender_key`. `plugins/test-runaway` is a plugin
that misbehaves on purpose: it proves loops and memory grabs are stopped.

```sh
scripts/build-plugins.sh sweep && cargo test wasm_host
```

## When something goes wrong

- **Not in Settings → Plugins?** The folder needs `plugin.wasm` at its top.
- **"Not running: …"** under its name: it failed to load, or it trapped. The
  text says which. Fix it and rebuild; Wax reloads it.
- **"action(s) refused":** it returned an action it wasn't granted. Ask for the
  permission in the manifest, rebuild, then switch it off and on to grant it.
- **Your `log` lines** show in the status bar and in Settings → Problems.
- **"stopped and was switched off"** means it ran out of fuel or memory, or
  panicked. A Rust panic is a trap.

## Versions

The interface is versioned (`wax:plugin@0.3.0`) and still changing: a plugin
built against one version won't load in a Wax built with another. When it
changes, rebuild against the new `wit/plugin.wit`.

- **v0.3** made commands run (keys, reader buttons, menus with choices and
  typed text), added `follow-up`, carried out `snooze-until`, added `utc-offset`,
  and gave `run-command` a `choice`.
- **v0.2** added sender views, `senders`, `knows-sender` and `file-sender`, and
  moved `on-message-received` ahead of showing new mail.
- **v0.1** was the first release.

## Checklist

- [ ] `id` in the manifest matches the folder name
- [ ] Asks only for the permissions it uses
- [ ] Every export implemented; unused ones return empty values
- [ ] No state kept in statics; settings in `config.toml`, memory in storage
- [ ] A `config.example.toml` with comments if it has settings
- [ ] Builds with `scripts/build-plugins.sh <name>`, and shows up switched off
