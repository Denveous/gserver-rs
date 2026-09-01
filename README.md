# GServer

GServer is a Rust game server for the Graal client protocol. It accepts game
clients, remote-control clients, NC clients, and the server-side NPC runtime
on one configured endpoint. It also supports WebSocket clients and the HTTP
API used by administration tools.

The server owns the live world: accounts, levels, boards, players, NPCs,
weapons, classes, flags, files, bans, list-server registration, and the
server-side script event loop. The wire format, packet identifiers, encryption
generations, and version-specific behavior are kept in the Rust implementation
and its protocol documentation.

## What the server provides

- TCP and WebSocket transport with packet framing, compression, and GEN_1
  through GEN_5 encryption.
- Player login, movement, chat, inventory, properties, flags, weapons,
  classes, warps, groups, and persistence.
- Remote control and NC administration, including account editing, rights,
  file transfer, bans, server options, and live player control.
- Level, map, board, link, sign, chest, item, baddy, horse, and NPC handling.
- Server-side NPC, weapon, class, and player scripts through the bundled Rust
  runtime crates.
- List-server communication, player and server listings, private-message
  routing, guild data, ban data, and staff activity.
- A signed HTTP API with embedded Swagger assets.

## Requirements

You need a current stable Rust toolchain with Cargo. The release build links
against the system SQLite and libcurl libraries because the scripting runtime
uses them for database and HTTP operations.

The server also needs a content directory containing the configured `config/`,
`accounts/`, `levels/`, `weapons/`, `classes/`, `npcs/`, and asset folders.
Public discovery and shared services require a reachable list server.

## Build and run

From the repository root:

```text
cargo build --release
./target/release/gameserver --server /path/to/server-content
```

For a development build:

```text
cargo run -- --server /path/to/server-content
```

The command-line options are `--server`, `--name`, `--port`, `--serverip`,
`--localip`, `--interface`, and `--silent`. The short single-dash forms are
accepted as well. `--silent` hides console output while the file logger keeps
writing `GServer.log`.

If `--server` is omitted, the process looks for `startupserver.txt` and then a
single entry under `servers/`. A server content directory can disable public
registration with this setting in `config/serveroptions.txt`:

```text
listserver = false
```

## Configuration

The most important files are:

`config/serveroptions.txt` controls the server name, ports, list-server
settings, gameplay options, script settings, and service switches.

`config/adminconfig.txt` defines remote-control access. Folder visibility and
read/write rules live in `config/foldersconfig.txt`.

`config/allowedversions.txt` restricts client versions. Local IP bans are in
`config/ipbans.txt`. Remote-control help and welcome text use `config/rchelp.txt`
and `config/rcmessage.txt`.

`serverflags.txt` stores server-wide flags. `servers.txt` describes optional
multiple server instances and their address overrides.

Account files use the standard server format and are stored below the content
directory. New accounts start without administrative rights.

## HTTP API and Swagger

The HTTP API shares the game listener. With a server running on port 14802,
the public status endpoint is:

```text
http://127.0.0.1:14802/api/v1/stats
```

`POST /api/v1/login` authenticates a staff account and returns a signed bearer
token. Protected script and file endpoints use that account's folder rights.
Swagger is available at `/swagger/` when `enableswagger = true`; its assets are
embedded in the binary, so the UI does not depend on a CDN.

## Documentation

The protocol and runtime guide is in [docs/GServer.md](docs/GServer.md). It
covers connection types, login, framing, encryption, packet families, world
state, administration, scripting, list-server traffic, files, accounts, and
the HTTP API.

## Repository layout

`src/model.rs` contains the server state, player and NPC behavior, world
loading, packet dispatch, administration, and script integration.

`src/network.rs`, `src/protocol.rs`, and `src/websocket.rs` implement transport,
wire encoding, compression, encryption, and WebSocket framing.

`src/config.rs` handles settings, content paths, files, permissions, and
logging. `src/http_api.rs` serves the shared HTTP API. `src/main.rs` is the
process entry point.

The Rust compiler and NPC runtime crates used by the server are under
`crates/`.

## Verification

```text
cargo fmt --check
cargo check --offline
cargo test --offline
cargo build --release --offline
```

The server should also be exercised with the intended client versions, remote
control tools, NC tools, list server, and content set before deployment.

## License

GPL-3.0-only.
