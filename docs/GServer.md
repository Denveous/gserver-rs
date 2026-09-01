# GServer guide

This guide explains how the Rust GServer is organised, how a client connects,
and where the protocol details live. It is written for people operating a
server, working on client compatibility, or extending the runtime.

The server is deliberately stateful. A connection is attached to a player
record, a player may be attached to a level, and level changes are propagated
to the other clients that can see that level. Account files and configuration
files are the persistent source of truth. The in-memory state is updated first
when an operation is live, then saved when the protocol requires persistence.

## Runtime shape

The process has one main server object. It owns configuration, the file system,
the active players, loaded levels, maps, classes, weapons, server flags, local
bans, and the list-server clients.

The game listener accepts all of the following on the configured port:

- Game clients, including the different client generations.
- Remote control clients, called RC clients in the protocol.
- NC clients that edit and inspect server-side NPC content.
- The optional NPC runtime connection.
- HTTP and WebSocket requests.

The listener identifies a request from its first bytes. HTTP and WebSocket
requests are routed to their respective handlers. A game connection begins
with a one-byte type followed by a newline. Once that first exchange is
complete, the connection uses the framing and encryption selected by the
client type and version.

The event loop performs short recurring work every second, minute, and five
minutes. It handles player timeouts, NPC timers, list-server maintenance,
autosaves, and periodic world updates without changing the order in which
packets are sent.

## Starting a server

Build the release binary from the repository root:

```text
cargo build --release
./target/release/gameserver --server /srv/graal/world
```

The `--server` value names the content directory. If it is omitted, the
process checks `startupserver.txt`, then uses the only entry under `servers/`
when there is exactly one. The other command-line settings are:

```text
--name       display name used by the server and list-server registration
--port       game and HTTP listener port
--serverip   public address announced to other services
--localip    address used for local service connections, or AUTO
--interface  local network interface selection
--silent     suppress console logging while retaining GServer.log
```

The single-dash spellings are accepted too. A server content directory can
disable list-server registration with:

```text
listserver = false
```

The release build needs the system SQLite and libcurl libraries. SQLite backs
script database calls. libcurl handles HTTP requests made by server-side
scripts.

## Configuration and content

The server reads settings from the content directory. The files most often
used during setup are:

`config/serveroptions.txt` contains server name, ports, list-server settings,
gameplay switches, script settings, and service toggles.

`config/adminconfig.txt` controls RC access. `config/foldersconfig.txt`
controls which folders an administrator can see, read, or write.

`config/allowedversions.txt` controls client versions. `config/ipbans.txt`
stores local IP ban data. `config/rchelp.txt` and `config/rcmessage.txt`
provide RC help and the RC welcome message.

`serverflags.txt` stores global flags. `servers.txt` describes optional server
instances and can override their bind and public addresses.

The usual content folders are:

```text
accounts/   account data and saved player properties
classes/    class scripts and class metadata
config/     server and administration settings
levels/     NW and Zelda level files
maps/       map, GMAP, and BigMap definitions
npcs/       database NPC files
weapons/    weapon scripts and compiled bytecode
```

Files are resolved through `FileSystem`. The resolver handles the configured
server root, the world folder alias, cached reads, line scanning, permissions,
and the file forms used by the RC browser. Path permissions use a segment-aware
pattern matcher, so a wildcard does not cross a directory separator.

## Connection types and login

The first line of a native connection is the client type. The type byte is
followed by `\n` and is not compressed or encrypted.

| Type | Meaning | Normal wire generation |
| ---: | --- | --- |
| 0 | Original game client | GEN_2 |
| 1 | RC client | GEN_3 |
| 2 | NPC runtime | GEN_3 |
| 3 | NC client | GEN_3 |
| 4 | Newer game client | GEN_4 |
| 5 | Current game client | GEN_5 |
| 6 | Current RC client | GEN_5 |
| 8 | Web client | no encryption |

After the type line, the login payload contains the client version and, where
needed, the account, password, and platform identity. NC and RC payloads have
their own field order. The login handler selects the encryption iterator,
loads the account, applies server staff rules, and assigns the connection a
player type.

A normal game login sends the following groups in order:

1. Signature and server capability packets.
2. Player login properties and flags.
3. Server flags, weapons, classes, and NPC-server status when applicable.
4. The current level, links, boards, signs, chests, items, baddies, and NPCs.
5. The server welcome and message text.
6. Existing player and staff entries visible to the new client.

RC and NC connections receive a control-oriented tail. They are not inserted
as physical occupants of a game level. The `(npcserver)` player is a special
server-owned entry and remains visible in the appropriate player lists.

When a player leaves, the server broadcasts the correct disconnect or delete
packets before removing the live record. Logout script events run as part of
that removal path. A failed socket write follows the same cleanup path as an
ordinary disconnect.

## Wire format

The packet helpers are in `src/network.rs` and `src/protocol.rs`. They should be
used instead of assembling numeric fields by hand.

The `Buffer` type reads and writes the protocol's little-endian values. The
important primitives are:

| Primitive | Description |
| --- | --- |
| GByte | One unsigned byte |
| GShort | Two-byte little-endian unsigned value |
| GInt | Four-byte little-endian signed or unsigned value |
| GInt5 | Five-byte variable integer used by rights and checksums |
| GString | NUL-terminated string |
| String8 | One-byte length followed by raw bytes |
| GToken | Protocol token encoding used by settings and text fields |

String8 length is a byte count, not a character count. Text sent through this
format must therefore follow the protocol's byte-length behavior. The Rust
implementation uses lossy conversion only at the text boundary where the
language requires valid UTF-8; packet lengths and binary fields remain byte
accurate.

The complete PLI and PLO numeric lists are public constants in
`src/protocol.rs`. Common client-to-server packets include:

```text
PLI_LEVELWARP       0
PLI_PLAYERPROPS     2
PLI_NPCPROPS        3
PLI_BADDYPROPS     15
PLI_FLAGSET        18
PLI_PRIVATEMESSAGE 28
PLI_TRIGGERACTION  38
PLI_RAWDATA        50
PLI_RC_CHAT        79
PLI_REQUESTTEXT   152
PLI_SENDTEXT      154
```

Common server-to-client packets include:

```text
PLO_LEVELBOARD      0
PLO_PLAYERPROPS     9
PLO_PLAYERWARP     14
PLO_DISCMESSAGE    16
PLO_SIGNATURE      25
PLO_FLAGSET        28
PLO_ADDPLAYER      55
PLO_DELPLAYER      56
PLO_RC_CHAT         74
PLO_SERVERTEXT      82
PLO_RAWDATA        100
PLO_FILE            102
PLO_NPCSERVERADDR   79
```

Some packets are wrapped in `PLO_RAWDATA` when the payload is itself a framed
or compressed data block. That wrapper must not be confused with a normal
packet containing arbitrary text.

## Compression and encryption

The encryption generation is selected during login and is part of the live
player state. The implementation preserves the iterator state across packets.
Do not reset an iterator when sending a packet or when flushing queued output.

The generations behave as follows:

- GEN_1 is plain transport and is used by the WebSocket game path.
- GEN_2 uses the original zlib framing without the later iterator step.
- GEN_3 uses zlib and inserts or removes one byte based on the packet iterator.
- GEN_4 uses the BZ2 packet form and the four-byte iterator transform.
- GEN_5 selects the packet form required by the client and uses the keyed
  iterator transform.

When outgoing packets are queued, the queue is flushed using the connection's
current generation. A server-side `sendCompress` operation combines queued
packets into the same compressed frame that a client expects. Queue state is
cleared only after a successful flush.

Compression errors and packets that exceed a client generation's limit are
logged using the same nonfatal path as the native server. Socket write errors
are different: they close the connection and trigger normal player cleanup.

## WebSocket transport

WebSocket requests are detected before native login parsing. The handshake
uses the standard `Sec-WebSocket-Key` calculation and returns a binary
connection. The framing code validates masking, payload lengths, control-frame
rules, close frames, ping/pong handling, and continuation frames.

The HTTP request bytes read while detecting the connection are replayed into
the selected handler. This matters when the first socket read contains both
the HTTP headers and the beginning of a native payload.

The server accepts persistent HTTP requests on the same listener. A `HEAD`
request receives the same headers as `GET` without the response body. A request
with `Connection: close` ends the connection after its response.

## World state

`Level` owns the loaded board and the objects currently in that level. Loading
supports NW and Zelda files, links, signs, chests, items, horses, baddies,
level NPCs, maps, and single-player instances. A level is cached by its cleaned
name and can be reloaded when a file changes or an RC upload replaces it.

The load path keeps the observable order important to clients:

1. Locate and read the level file.
2. Parse tile layers and board metadata.
3. Parse object records in file order.
4. Attach NPCs and baddies to the loaded level.
5. Insert the level in the server cache.
6. Send level data to clients that need it.

Board changes are broadcast as board-modify or board-layer packets depending
on the client generation. Precise movement properties are sent to clients
that support them, while older clients receive the corresponding compact
coordinates. Mixed-version sessions are handled per recipient.

Map, GMAP, and BigMap records point to level names. Group maps filter visible
players by group membership. Single-player levels are cloned per player so
their board and NPC mutations do not leak to other sessions.

## Players, NPCs, and baddies

Player properties are stored in a fixed property array because property order
is part of the client protocol. Login, local updates, RC edits, and replication
all use the same property indices with version-specific inclusion rules.

An NPC has character properties, position, script state, flags, attributes,
weapons, and an optional level owner. Database NPCs are loaded from `npcs/`;
level NPCs are loaded from the level data or created through NC operations.
Changing a database NPC script clears its VM state and schedules its creation
event again.

Baddies use the server's baddy type, position, image, power, health, timeout,
and item-drop rules. Position values are clamped to the protocol's valid range.
Add, property, hurt, and delete operations are broadcast in the order required
by connected clients.

## Flags, weapons, classes, and scripts

Server flags are global and are broadcast to the relevant clients. Client flags
belong to one player. The flag name prefixes determine which store is used.
Flag set and delete operations also invoke the matching server-side event
when a script is listening.

Weapons and classes have both client-visible data and server-side script data.
Weapon files may contain a client-side section and a server-side section. The
compiler stores bytecode alongside the source when the configuration calls for
it. A compile diagnostic is sent to the controlling NC or RC connection and
does not silently become a successful update.

The NPC runtime exposes players, levels, NPCs, server flags, server options,
files, sockets, HTTP requests, SQLite, timers, and the supported script events.
Results are applied in the server thread. Stale scheduled events are ignored,
and object ownership is preserved when an event invokes another event.

Important event families include:

```text
onCreated, onPlayerLogin, onPlayerLogout, onPlayerChat
onPlayerTouchsMe, onActionServerside, onAllRCChat
onTimeout, onWeaponFired, onWeaponCreated
```

The event dispatcher preserves the distinction between a displayed RC chat
line, a slash command, and server-generated administration output. Slash
commands are handled by the RC command parser and are not echoed as ordinary
chat.

## RC and NC administration

RC access is controlled by the account's staff status, administrative level,
IP rules, and folder rights. The server checks the required right before each
operation. A visible file is not automatically readable, and a readable file
is not automatically writable.

RC operations include:

- Open, edit, reset, warp, disconnect, ban, and inspect players.
- Add, delete, list, and edit accounts, comments, rights, guilds, weapons,
  classes, and server flags.
- Browse, upload, download, move, rename, and delete files.
- Change server options and reload levels, classes, weapons, or NPC data.

NC operations focus on live content. NC clients can list and edit database
NPCs, add or remove classes and weapons, inspect local NPCs, warp or delete
NPCs, set flags, and request level lists. A successful script update is saved,
compiled when required, applied to the live object, and announced to the
appropriate control connections.

The file browser uses the raw packet payload for filenames and directory names.
Large uploads use the large-file start, data, and end packets. The server
does not delete the upload state until the final save succeeds.

## List-server and text services

Each enabled list-server connection maintains its own socket, encryption
iterator, description, language, version, address, port, and server flags.
Timed maintenance reconnects or refreshes the connection without disturbing
the player listener.

The requestText and sendText packets carry service commands. The server routes
account checks, player lists, private-message server lists, guild data, ban
data, staff activity, and server registration through this path. When no list
server is available, local fallbacks are used only for the commands that have
an explicit local behavior.

Incoming service text is parsed by command and comma-separated fields. The
payload after the command remains raw when the command defines a raw field.
This keeps account names, comments, reasons, and binary-compatible values
intact.

## HTTP API

The API is served on the same TCP endpoint as the game protocol. The request
parser supports persistent HTTP/1.1 requests, exact method handling, HEAD
responses, and the embedded Swagger files.

Useful routes are:

```text
GET  /api
GET  /api/v1/stats
POST /api/v1/login
GET  /api/v1/scripts/...
GET  /api/v1/files/...
POST /api/v1/files/...
GET  /swagger/
GET  /swagger/v1/swagger.json
```

`/api/v1/stats` is public. The login route checks a staff account and returns
a signed bearer token. Protected routes validate the token, confirm its
expiration, and apply the account's file or script permission. Unknown API
routes use the same JSON error shape as the rest of the API; unknown non-API
routes use a plain HTTP 404 response.

Swagger is enabled by `enableswagger = true`. The assets are embedded at build
time, so the server does not need a source checkout or an external CDN at
runtime.

## Persistence and shutdown

Account changes are written to the configured account shard. The account name
determines its directory and the special runtime account has its own stable
path. Player logout saves the fields covered by the account format and then
removes the live player from the server.

The one-second timer performs short-lived maintenance. The one-minute timer
saves players and runtime data. The five-minute timer performs the slower
server and list-server maintenance. A clean shutdown stops the listener,
disconnects managed sockets, stops the NPC runtime, flushes the logger, and
leaves account data in its last successfully saved state.

## Source map

The main Rust modules have narrow responsibilities:

`src/main.rs` parses process options, installs signal handling, selects the
server directory, and starts the runtime.

`src/model.rs` contains the server, player, level, NPC, list-server, control,
file, persistence, and script integration logic.

`src/network.rs` contains buffers, encryption, compression, and socket
management. `src/protocol.rs` contains packet identifiers and shared protocol
constants. `src/websocket.rs` contains WebSocket handshakes and frames.

`src/config.rs` handles settings, file resolution, permissions, and logging.
`src/http_api.rs` handles HTTP requests, authentication, API responses, and
embedded Swagger assets.

The reusable Rust compiler and NPC runtime crates are in `crates/` in the
published tree.

## Extending the server safely

When adding a packet handler, first identify the client types and versions that
can send it. Parse fields with `Buffer`, validate lengths before indexing, and
return the same success or failure result used by neighboring handlers.

When changing live state, update the owning object while holding its state
lock, then release that lock before sending to other players. Use the existing
broadcast helpers so recipient filtering and client-version selection remain
consistent.

When changing persistence, test both a live player and an offline account.
When changing a timer, test the exact boundary as well as the normal interval.
When changing a packet, add a byte-level test and cover at least one older and
one current client generation.

The protocol constants and the implementation are the authoritative references
for behavior. This guide explains the surrounding contracts without replacing
those byte-level definitions.
