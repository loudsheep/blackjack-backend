# Blackjack Backend

A robust, real-time multiplayer Blackjack game server written in Rust. This backend powers the game logic, state management, and communication for a Blackjack platform.

## Tech Stack
*   **Language:** Rust (2021 Edition)
*   **Framework:** Axum (Web & WebSockets)
*   **Runtime:** Tokio
*   **Serialization:** Serde JSON

## Setup & Running

1.  **Clone & Run:**
    ```bash
    cargo run
    ```
    Server starts at `http://127.0.0.1:3000`.

2.  **Configuration (.env):**
    ```env
    APP_ADDRESS=0.0.0.0:3000
    RUST_LOG=blackjack_backend=debug
    ```

## Frontend Integration Guide

This backend uses a persistent WebSocket connection. State is authoritative on the server.

### 1. Game Creation (REST)
**POST** `/game/create`  
Create a new room before connecting.

**Request:**
```json
{
  "settings": {
    "initial_chips": 1000,
    "max_players": 5,
    "deck_count": 6,
    "approval_required": false,
    "chat_enabled": true
  }
}
```

**Response:**
```json
{ "game_id": "849201" }
```

### 2. Connection & Reconnection (WebSocket)
**Endpoint:** `ws://<host>/ws/<game_id>`

The server supports session persistence. You **must** store the credentials received in `JoinedLobby` to allow players to refresh the page without losing their spot.

*   **New Session:**
    Connect to `ws://localhost:3000/ws/849201`.
*   **Reconnection:**
    Append credentials to the URL:
    `ws://localhost:3000/ws/849201?player_id=<UUID>&secret=<SECRET>`

**Status Codes:**
*   `101`: Connected.
*   `404`: Game not found (ended or wrong ID).
*   `403`: Game is full (and you are not reconnecting).

### 3. Client Implementation Flow
1.  **Connect** via WebSocket.
2.  If this is a fresh session, send `JoinGame`.
3.  Listen for `JoinedLobby`. **Save `your_id` and `secret` to LocalStorage immediately.**
4.  Listen for `GameStateSnapshot` to render the UI.
5.  On page reload, read LocalStorage. If keys exist, connect using the Reconnection URL.
6.  If reconnection fails (socket closes or Error received), clear LocalStorage and connect normally.

---

## Protocol Reference

All WebSocket messages are JSON.

### Data Structures

**Card**
```json
{ "suit": "Hearts", "rank": "Ace" }
```
*   **Suits:** `Hearts`, `Diamonds`, `Clubs`, `Spades`
*   **Ranks:** `Two`..`Ten`, `Jack`, `Queen`, `King`, `Ace`

**Player**
```json
{
  "id": "uuid-string",
  "name": "Alice",
  "chips": 1000,
  "hands": [ ... ],       // See Hand structure
  "active_hand_index": 0, // Index of the hand currently being acted on
  "status": "Playing",    // Spectating, Sitting, Playing, PendingApproval
  "is_admin": true,       // Can perform admin actions
  "is_connected": true    // True if connected, false if offline/disconnected
}
```

**Hand**
```json
{
  "cards": [ { "suit": "Spades", "rank": "Ten" }, ... ],
  "bet": 100,
  "status": "Playing" // Playing, Stood, Busted, Blackjack, Doubled
}
```

**GameSettings**
```json
{
  "initial_chips": 1000,
  "max_players": 5,
  "deck_count": 1,
  "approval_required": false,
  "chat_enabled": true
}
```

**GamePhase**
Values: `Lobby`, `Betting`, `Playing`, `DealerTurn`, `Payout`, `GameOver`

---

### Client Messages (Send)

Wrap all messages in: `{ "action": "Name", "payload": { ... } }`

| Action | Payload JSON | Description |
| :--- | :--- | :--- |
| **JoinGame** | `{ "username": "Bob" }` | Register as a player. Required if not reconnecting. |
| **PlaceBet** | `{ "amount": 50 }` | Bet chips. Valid only in `Betting` phase. |
| **GameAction** | `{ "action_type": "Hit" }` | Types: `Hit`, `Stand`, `Double`, `Split`. Valid in `Playing` phase. |
| **Chat** | `{ "message": "Hi" }` | Broadcast text to all players. |
| **StartGame** | `null` | **(Admin)** Force start the game or skipped current player turn. |
| **NextRound** | `null` | **(Admin)** Phase: `Payout` -> `Betting`. |
| **ApprovePlayer**| `{ "player_id": "..." }` | **(Admin)** Allow a `PendingApproval` player to join. |
| **KickPlayer** | `{ "player_id": "..." }` | **(Admin)** Remove and disconnect a player. |
| **UpdateSettings**| `{ "settings": { ... } }` | **(Admin)** Change rules mid-game. |
| **AdminUpdateBalance** | `{ "target_id": "...", "change_chips": 500 }` | **(Admin)** Modify player chips (use negative to deduct). |
| **Ping** | `null` | Check latency/connection. Server replies with `Pong`. |

**Note on Phases:**
1.  **Lobby**: Players join. Admin starts game (`StartGame`).
2.  **Betting**: Players place bets (Status -> `Playing`). Non-betters stay `Sitting`.
    *   System Auto-Start: If **everyone** eligible and **connected** has bet. Offline players are ignored.
    *   Admin Force-Start: If some have bet, Admin can `StartGame` to proceed. Non-betters skip the round.
3.  **Playing**: Players take actions.
    *   Admin can `StartGame` to force-stand the current player (timeout).
4.  **DealerTurn**: Automatic.
5.  **Payout**: Results calculated.
6.  **NextRound**: Admin resets to `Betting` (`NextRound`).

---

### Server Messages (Receive)

Wrapped in: `{ "event": "Name", "data": { ... } }`

#### 1. GameStateSnapshot
Sent on **any** change. This is the **entire** state needed to render the game.
```json
{
  "event": "GameStateSnapshot",
  "data": {
    "phase": "Playing",
    "dealer_hand": [ {"suit": "Clubs", "rank": "Five"} ], // First card hidden if playing
    "players": [ ... ], // Array of Player objects
    "deck_remaining": 42,
    "current_turn_player_id": "uuid-string", // Whose turn is it? (null if not Playing)
    "settings": { ... }
  }
}
```

**Hand Status Values:**
`Playing`, `Stood`, `Busted`, `Blackjack`, `Doubled`, `Won`, `Lost`, `Push`

#### 2. JoinedLobby (Crucial)
Sent only to YOU after a successful join or reconnect.
```json
{
  "event": "JoinedLobby",
  "data": {
    "game_id": "123456",
    "your_id": "uuid-string",
    "secret": "KEEP_THIS_SECRET_uuid", // Save this for reconnection!
    "is_admin": false
  }
}
```

#### 3. Error
Sent when an action fails.
```json
{
  "event": "Error",
  "data": { "msg": "Not enough chips to split." }
}
```

#### 4. PlayerRequest
Sent to **Admins only** when `approval_required` is true and someone joins.
```json
{
  "event": "PlayerRequest",
  "data": { "id": "uuid", "name": "Stranger" }
}
```

#### 5. ChatBroadcast
```json
{
  "event": "ChatBroadcast",
  "data": { "from": "Alice", "msg": "gg" }
}
```

#### 6. Pong
Response to **Ping**.
```json
{
  "event": "Pong",
  "data": null
}
```

#### 7. Kicked
Sent when you are removed from the game by an admin. The connection is closed immediately after.
```json
{
  "event": "Kicked",
  "data": null
}
```

### Message Scoping
- **Error Messages**: Most error messages (e.g. "Not enough chips", "It is not your turn") are now sent only to the relevant player (unicast), instead of being broadcast to the entire room.
- **Game State**: Game state updates and chat messages are broadcast to all connected clients.
