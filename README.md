# Blackjack Backend

A robust, real-time multiplayer Blackjack game server written in Rust. This backend powers the game logic, state management, and communication for a Blackjack platform, designed to be consumed by a web or mobile frontend.

## Tech Stack

*   **Language:** Rust (2021 Edition)
*   **Web Framework:** [Axum](https://github.com/tokio-rs/axum)
*   **Async Runtime:** Tokio
*   **WebSockets:** Native Axum WebSocket integration
*   **Serialization:** Serde & Serde JSON
*   **State Management:** Actor-like pattern using Tokio channels (`mpsc` & `broadcast`)

## Setup Guide

### Prerequisites
*   [Rust & Cargo](https://rustup.rs/) installed.

### Installation & Running

1.  **Clone the repository:**
    ```bash
    git clone https://github.com/yourusername/blackjack-backend.git
    cd blackjack-backend
    ```

2.  **Configuration (Optional):**
    You can create a `.env` file in the root directory to override defaults:
    ```env
    # default: 127.0.0.1:3000
    APP_ADDRESS=0.0.0.0:8080 
    # default: blackjack_backend=debug
    RUST_LOG=blackjack_backend=info 
    ```

3.  **Run the server:**
    ```bash
    cargo run
    ```

The server will start (default: `http://127.0.0.1:3000`).

## Frontend Documentation

The backend exposes a WebSocket interaction model. The application state is authoritative on the server.

### Creating a Game
**URL:** `POST http://<host>/game/create`

**Request Body:**
```json
{
  "initial_chips": 1000,
  "max_players": 5,
  "deck_count": 1,
  "approval_required": false,
  "chat_enabled": true
}
```

**Response:**
```json
{
  "game_id": "123456"
}
```

### Connection
**URL:** `ws://<host>/ws/<game_id>`

*   `game_id`: A unique string identifier for the room. You must create the game using the `/game/create` endpoint before connecting. If the room doesn't exist, the connection will be rejected with a `404 Not Found`, if room is full, it will be rejected with a `403 Forbidden`.

### Protocol
All messages are JSON objects.

#### Client Messages (Frontend -> Backend)
Send these messages with the structure `{"action": "ActionName", "payload": { ... }}`.

| Action | Payload | Description |
| :--- | :--- | :--- |
| `JoinGame` | `{ "username": "Bob" }` | Join the lobby of the connected game. |
| `StartGame` | `null` | (Admin) Start the game, moving from Lobby to Betting phase. |
| `NextRound` | `null` | (Admin) Start the next round, moving from Payout to Betting phase. |
| `PlaceBet` | `{ "amount": 100 }` | Place chips during the `Betting` phase. |
| `GameAction` | `{ "action_type": "Hit" }` | Perform a move. Types: `Hit`, `Stand`, `Double`, `Split`. |
| `Chat` | `{ "message": "Hello" }` | Send a chat message. |
| `ApprovePlayer` | `{ "player_id": "uuid" }` | (Admin) Approve a player in `PendingApproval` state. |
| `KickPlayer` | `{ "player_id": "uuid" }` | (Admin) Kick a player from the game. |
| `UpdateSettings` | `{ "settings": { ... } }` | (Admin) Update game settings mid-game. |
| `AdminUpdateBalance` | `{ "target_id": "uuid", "change_chips": 100 }` | (Admin) Add or remove chips from a player. Negative values remove chips. |

**Settings Object:**
```json
{
  "initial_chips": 1000,
  "max_players": 5,
  "deck_count": 1,
  "approval_required": false,
  "chat_enabled": true
}
```

#### Server Messages (Backend -> Frontend)
The server broadcasts these events with the structure `{"event": "EventName", "data": { ... }}`.

**1. `GameStateSnapshot`**
Sent whenever the game state changes. This is the source of truth for rendering.
```json
{
  "event": "GameStateSnapshot",
  "data": {
    "phase": "Betting", // Lobby, Betting, Playing, DealerTurn, Payout, GameOver
    "dealer_hand": [ {"suit": "Spades", "rank": "Ace"} ], // Hidden/Truncated if playing
    "players": [
      {
        "id": "uuid-string",
        "name": "Alice",
        "chips": 1000,
        "status": "Playing",
        "hands": [ ... ],
        "is_admin": true
      }
    ],
    "deck_remaining": 48,
    "current_turn_player_id": "uuid-string", // Null if not playing phase
    "settings": { ... } // Current game settings
  }
}
```

**2. `JoinedLobby`**
Confirmation sent to a user upon successful join.
```json
{
  "event": "JoinedLobby",
  "data": {
    "game_id": "room1",
    "your_id": "uuid-string",
    "is_admin": true
  }
}
```

**3. `ChatBroadcast`**
```json
{
  "event": "ChatBroadcast",
  "data": {
    "from": "Alice",
    "msg": "Good luck!"
  }
}
```

**4. `PlayerRequest`**
Sent to admins when a player joins a room with `approval_required` enabled.
```json
{
  "event": "PlayerRequest",
  "data": {
    "id": "uuid-string",
    "name": "Charlie"
  }
}
```

**5. `Error`**
Sent when an invalid action is attempted or a system error occurs.
```json
{
  "event": "Error",
  "data": { "msg": "Not enough chips to double down." }
}
```
