use crate::game::types::*;
use crate::messages::{ActionType, BroadcastMessage, ClientMessage, ServerMessage};
use rand::seq::SliceRandom;
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

pub struct GameActor {
    game_id: String,
    settings: GameSettings,
    phase: GamePhase,
    deck: Vec<Card>,
    players: Vec<Player>, // List of all players
    dealer_hand: Vec<Card>,
    turn_index: usize, // Who is playing right now?

    // connection_id -> player_id
    connection_map: std::collections::HashMap<Uuid, Uuid>,

    // Channels
    receiver: mpsc::Receiver<(Uuid, ClientMessage)>, // We need to know WHO sent the msg
    sender: broadcast::Sender<BroadcastMessage>,
    player_count_ref: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    cleanup_sender: mpsc::Sender<String>,
}

impl GameActor {
    pub fn new(
        game_id: String,
        settings: GameSettings,
        receiver: mpsc::Receiver<(Uuid, ClientMessage)>,
        sender: broadcast::Sender<BroadcastMessage>,
        player_count_ref: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        cleanup_sender: mpsc::Sender<String>,
    ) -> Self {
        let mut actor = Self {
            game_id,
            settings,
            phase: GamePhase::Lobby,
            deck: Vec::new(),
            players: Vec::new(),
            dealer_hand: Vec::new(),
            turn_index: 0,
            connection_map: std::collections::HashMap::new(),
            receiver,
            sender,
            player_count_ref,
            cleanup_sender,
        };
        actor.init_deck();
        actor
    }

    pub async fn run(&mut self) {
        loop {
            // If there are no active connections, wait with a timeout.
            // If the timeout triggers, clean up and exit.
            let msg_option = if self.connection_map.is_empty() {
                // 60 seconds grace period for reconnections/initial join
                match tokio::time::timeout(std::time::Duration::from_secs(60), self.receiver.recv())
                    .await
                {
                    Ok(res) => res,
                    Err(_) => {
                        // Timeout reached, cleanup
                        tracing::info!("Game {} timeout due to inactivity", self.game_id);
                        let _ = self.cleanup_sender.send(self.game_id.clone()).await;
                        return;
                    }
                }
            } else {
                self.receiver.recv().await
            };

            if let Some((conn_id, msg)) = msg_option {
                match msg {
                    ClientMessage::JoinGame { username } => {
                        self.handle_join(conn_id, username);
                    }
                    ClientMessage::Reconnect { player_id, secret } => {
                        self.handle_reconnect(conn_id, player_id, secret);
                    }
                    ClientMessage::Disconnect => {
                        self.handle_disconnect(conn_id);
                    }
                    other => {
                        if let Some(&player_id) = self.connection_map.get(&conn_id) {
                            self.handle_game_message(player_id, other);
                        }
                    }
                }
            } else {
                // Channel closed
                break;
            }
        }
    }

    fn handle_game_message(&mut self, player_id: Uuid, msg: ClientMessage) {
        match msg {
            ClientMessage::PlaceBet { amount } => {
                self.handle_bet(player_id, amount);
            }

            ClientMessage::GameAction { action_type } => {
                self.handle_action(player_id, action_type);
            }

            ClientMessage::ApprovePlayer {
                player_id: target_id,
            } => {
                self.handle_approve(player_id, target_id);
            }

            ClientMessage::KickPlayer {
                player_id: target_id,
            } => {
                self.handle_kick(player_id, target_id);
            }

            ClientMessage::UpdateSettings { settings } => {
                self.handle_update_settings(player_id, settings);
            }

            ClientMessage::AdminUpdateBalance {
                target_id,
                change_chips,
            } => {
                self.handle_admin_update_balance(player_id, target_id, change_chips);
            }

            ClientMessage::Chat { message } => {
                self.handle_chat(player_id, message);
            }

            ClientMessage::StartGame => {
                self.handle_start_game(player_id);
            }

            ClientMessage::NextRound => {
                self.handle_next_round(player_id);
            }

            ClientMessage::Ping => {
                if let Some(conn_id) = self
                    .connection_map
                    .iter()
                    .find(|(_, val)| **val == player_id)
                    .map(|(c, _)| *c)
                {
                    self.send_to(conn_id, ServerMessage::Pong);
                }
            }

            _ => {} // Handled in run()
        }
    }

    fn init_deck(&mut self) {
        self.deck.clear();
        for _ in 0..self.settings.deck_count {
            self.deck.extend(Card::new_deck());
        }
        let mut rng = rand::rng();
        self.deck.shuffle(&mut rng);
    }

    fn is_admin(&self, player_id: Uuid) -> bool {
        self.players
            .iter()
            .find(|p| p.id == player_id)
            .map(|p| p.is_admin)
            .unwrap_or(false)
    }

    // Helper functions
    fn broadcast(&self, msg: ServerMessage) {
        let _ = self.sender.send(BroadcastMessage {
            target: None,
            message: msg,
        });
    }

    fn send_to(&self, conn_id: Uuid, msg: ServerMessage) {
        let _ = self.sender.send(BroadcastMessage {
            target: Some(conn_id),
            message: msg,
        });
    }

    fn send_error(&self, player_id: Uuid, msg: impl Into<String>) {
        if let Some(conn_id) = self
            .connection_map
            .iter()
            .find(|(_, val)| **val == player_id)
            .map(|(c, _)| *c)
        {
            self.send_to(conn_id, ServerMessage::Error { msg: msg.into() });
        }
    }

    fn update_player_count(&self) {
        let count = self.players.iter().filter(|p| p.is_connected).count();
        self.player_count_ref
            .store(count, std::sync::atomic::Ordering::Relaxed);
    }

    fn handle_join(&mut self, conn_id: Uuid, username: String) {
        if self.connection_map.contains_key(&conn_id) {
            return;
        }

        if self.players.len() >= self.settings.max_players {
            self.send_to(
                conn_id,
                ServerMessage::Error {
                    msg: "Game is full".to_string(),
                },
            );
            return;
        }

        let player_id = Uuid::new_v4();
        let secret = Uuid::new_v4().to_string();
        self.connection_map.insert(conn_id, player_id);

        let is_first = self.players.is_empty();
        let status = if self.settings.approval_required && !is_first {
            PlayerStatus::PendingApproval
        } else {
            PlayerStatus::Spectating
        };

        if status == PlayerStatus::PendingApproval {
            let msg = ServerMessage::PlayerRequest {
                id: player_id,
                name: username.clone(),
            };
            self.broadcast(msg); // Admins should listen for this
        }

        self.players.push(Player {
            id: player_id,
            name: username,
            chips: self.settings.initial_chips,
            hands: vec![],
            active_hand_index: 0,
            status: status.clone(),
            is_admin: is_first,
            secret: secret.clone(),
            is_connected: true,
        });

        self.update_player_count();

        self.send_to(
            conn_id,
            ServerMessage::JoinedLobby {
                game_id: self.game_id.clone(),
                your_id: player_id,
                secret: secret,
                is_admin: is_first,
            },
        );

        // If game is in lobby, just broadcast join
        // If game is in progress (Betting, Playing, Payout), newcomer is Spectating
        // but state should reflect that. Current implementation defaults to Spectating which is correct.
        self.broadcast_state();
    }

    fn handle_reconnect(&mut self, conn_id: Uuid, player_id: Uuid, secret: String) {
        if self.connection_map.contains_key(&conn_id) {
            return;
        }

        let mut success = false;
        let mut is_admin_val = false;
        let mut secret_val = String::new();
        let mut error_msg = None;

        if let Some(player) = self.players.iter_mut().find(|p| p.id == player_id) {
            if player.secret == secret {
                self.connection_map.insert(conn_id, player_id);
                player.is_connected = true;
                success = true;
                is_admin_val = player.is_admin;
                secret_val = player.secret.clone();
            } else {
                error_msg = Some("Invalid secret");
            }
        } else {
            error_msg = Some("Player not found");
        }

        if success {
            self.update_player_count();

            self.send_to(
                conn_id,
                ServerMessage::JoinedLobby {
                    game_id: self.game_id.clone(),
                    your_id: player_id,
                    secret: secret_val,
                    is_admin: is_admin_val,
                },
            );
            self.broadcast_state();
        } else if let Some(msg) = error_msg {
            self.send_to(conn_id, ServerMessage::Error { msg: msg.into() });
        }
    }

    fn handle_disconnect(&mut self, conn_id: Uuid) {
        if let Some(player_id) = self.connection_map.remove(&conn_id) {
            let still_connected = self.connection_map.values().any(|&pid| pid == player_id);
            if let Some(player) = self.players.iter_mut().find(|p| p.id == player_id) {
                if !still_connected {
                    player.is_connected = false;
                }
            }
            self.update_player_count();
            // Don't remove player immediately.
            self.broadcast_state();
        }
    }

    fn handle_approve(&mut self, admin_id: Uuid, target_id: Uuid) {
        if !self.is_admin(admin_id) {
            self.send_error(admin_id, "Only admins can approve players.");
            return;
        }

        if let Some(player) = self.players.iter_mut().find(|p| p.id == target_id) {
            if player.status == PlayerStatus::PendingApproval {
                player.status = PlayerStatus::Spectating;
                self.broadcast_state();
            }
        }
    }

    fn handle_kick(&mut self, admin_id: Uuid, target_id: Uuid) {
        if !self.is_admin(admin_id) {
            self.send_error(admin_id, "Only admins can kick players.");
            return;
        }

        if admin_id == target_id {
            self.send_error(admin_id, "You cannot kick yourself.");
            return;
        }

        // Find all connections for this player and disconnect them
        let connections: Vec<Uuid> = self
            .connection_map
            .iter()
            .filter(|&(_, &pid)| pid == target_id)
            .map(|(&cid, _)| cid)
            .collect();

        for cid in connections {
            self.send_to(cid, ServerMessage::Kicked);
            self.connection_map.remove(&cid);
        }

        // Remove player
        if let Some(pos) = self.players.iter().position(|p| p.id == target_id) {
            self.players.remove(pos);
            self.update_player_count();
            self.broadcast_state();
        }
    }

    fn handle_update_settings(&mut self, admin_id: Uuid, settings: GameSettings) {
        if !self.is_admin(admin_id) {
            self.send_error(admin_id, "Only admins can update settings.");
            return;
        }
        self.settings = settings;
        self.broadcast_state();
    }

    fn handle_admin_update_balance(&mut self, admin_id: Uuid, target_id: Uuid, change_chips: i32) {
        if !self.is_admin(admin_id) {
            self.send_error(admin_id, "Only admins can update player balances.");
            return;
        }

        if let Some(player) = self.players.iter_mut().find(|p| p.id == target_id) {
            if change_chips < 0 {
                let deduction = (-change_chips) as u32;
                if player.chips >= deduction {
                    player.chips -= deduction;
                } else {
                    player.chips = 0;
                }
            } else {
                player.chips += change_chips as u32;
            }
            self.broadcast_state();
        }
    }

    fn handle_chat(&mut self, player_id: Uuid, msg: String) {
        if !self.settings.chat_enabled {
            self.send_error(player_id, "Chat is currently disabled.");
            return;
        }

        let sender_name = self
            .players
            .iter()
            .find(|p| p.id == player_id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "Unknown".to_string());

        self.broadcast(ServerMessage::ChatBroadcast {
            from: sender_name,
            msg,
        });
    }

    fn handle_start_game(&mut self, player_id: Uuid) {
        if !self.is_admin(player_id) {
            self.send_error(player_id, "Only admins can start the game.");
            return;
        }

        match self.phase {
            GamePhase::Lobby => self.start_betting_phase(),
            GamePhase::Betting => self.start_action_phase(),
            GamePhase::Playing => {
                // Admin force skip current player
                self.force_stand_current_player();
            }
            _ => {
                self.send_error(player_id, "Invalid phase for StartGame command.");
            }
        }
    }

    fn force_stand_current_player(&mut self) {
        if let Some(player) = self.players.get_mut(self.turn_index) {
            // Mark all active hands as Stood
            for hand in player.hands.iter_mut() {
                if hand.status == HandStatus::Playing {
                    hand.status = HandStatus::Stood;
                }
            }
        }
        self.advance_turn();
    }

    fn handle_next_round(&mut self, player_id: Uuid) {
        if !self.is_admin(player_id) {
            self.send_error(player_id, "Only admins can start the next round.");
            return;
        }

        match self.phase {
            GamePhase::Payout | GamePhase::GameOver => self.start_betting_phase(),
            _ => {
                self.send_error(player_id, "Next round can only be started from Payout or GameOver phase.");
            }
        }
    }

    fn handle_bet(&mut self, player_id: Uuid, amount: u32) {
        if self.phase != GamePhase::Betting {
            self.send_error(player_id, "Bets can only be placed during the Betting phase.");
            return;
        }

        let mut status_changed = false;

        if let Some(player) = self.players.iter_mut().find(|p| p.id == player_id) {
            if player.status == PlayerStatus::PendingApproval {
                self.send_error(player_id, "You must be approved before playing.");
                return;
            }

            // Only switch from Sitting request -> Playing when betting
            if player.status != PlayerStatus::Sitting && player.status != PlayerStatus::Spectating {
                self.send_error(player_id, "You cannot place a bet right now.");
                return;
            }

            if player.chips >= amount {
                player.chips -= amount;
                player.status = PlayerStatus::Playing;

                player.hands = vec![Hand {
                    cards: Vec::new(),
                    bet: amount,
                    status: HandStatus::Playing,
                }];
                player.active_hand_index = 0;
                status_changed = true;
            } else {
                self.send_error(player_id, "Not enough chips to place bet.");
                return;
            }
        }

        if !status_changed {
            return;
        }

        // Check if all ELIGIBLE players have bet.
        // Who is eligible?
        // - Anyone who is 'Sitting' or 'Playing'.
        // - 'Spectating' implicitly means 'Sitting' in this logic or 'Pending'.
        // Actually, if we require ALL players in the room to bet before auto-start,
        // then one person sitting out blocks the game.
        // Flow Requirement: "if all have betted the system automatically starts".
        // This implies if there are 3 people, and 3 people bet -> start.
        // If 3 people, 2 bet, 1 sits -> Wait for admin or wait for 3rd?
        // User said: "not betting players do not participate... game should not await their actions"
        // So automatic start only happens if EVERYONE currently connected (and approved) has placed a bet.

        let all_ready = self.players.iter().all(|p| {
            !p.is_connected
                || p.status == PlayerStatus::Playing
                || p.status == PlayerStatus::PendingApproval // Pending don't count
            // If someone is Sitting/Spectating, they haven't bet yet.
            // If we have any 'Sitting' players, we are NOT ready for auto-start.
        });

        if all_ready {
            self.start_action_phase();
        } else {
            self.broadcast_state();
        }
    }

    fn handle_action(&mut self, player_id: Uuid, action: ActionType) {
        if self.phase != GamePhase::Playing {
            self.send_error(player_id, "Actions can only be performed during the Playing phase.");
            return;
        }

        let is_turn = self
            .get_current_player()
            .map(|p| p.id == player_id)
            .unwrap_or(false);

        if !is_turn {
            self.send_error(player_id, "It is not your turn.");
            return;
        }

        let mut action_result = ActionResult::None;

        if let Some(player) = self.players.get(self.turn_index) {
            if let Some(hand) = player.hands.get(player.active_hand_index) {
                match action {
                    ActionType::Hit => action_result = ActionResult::Hit,
                    ActionType::Stand => action_result = ActionResult::Stand,
                    ActionType::Double => {
                        if player.chips >= hand.bet {
                            action_result = ActionResult::Double(hand.bet);
                        } else {
                            self.send_error(player_id, "Not enough chips to double down.");
                        }
                    }
                    ActionType::Split => {
                        if hand.cards.len() == 2
                            && player.chips >= hand.bet
                            && hand.cards[0].rank == hand.cards[1].rank
                        {
                            action_result = ActionResult::Split(hand.bet);
                        } else {
                            self.send_error(player_id, "Cannot split: need pair and enough chips.");
                        }
                    }
                }
            }
        }

        let mut should_advance = false;

        match action_result {
            ActionResult::Hit => {
                if let Some(card) = self.draw_card() {
                    if let Some(player) = self.players.get_mut(self.turn_index) {
                        if let Some(hand) = player.hands.get_mut(player.active_hand_index) {
                            hand.cards.push(card);
                            if calculate_hand_value(&hand.cards) > 21 {
                                hand.status = HandStatus::Busted;
                                should_advance = true;
                            }
                        }
                    }
                }
            }
            ActionResult::Stand => {
                if let Some(player) = self.players.get_mut(self.turn_index) {
                    if let Some(hand) = player.hands.get_mut(player.active_hand_index) {
                        hand.status = HandStatus::Stood;
                        should_advance = true;
                    }
                }
            }
            ActionResult::Double(bet_amount) => {
                if let Some(player) = self.players.get_mut(self.turn_index) {
                    player.chips -= bet_amount;
                }

                let card = self.draw_card();

                if let Some(c) = card {
                    if let Some(player) = self.players.get_mut(self.turn_index) {
                        if let Some(hand) = player.hands.get_mut(player.active_hand_index) {
                            hand.bet += bet_amount;
                            hand.cards.push(c);
                            if calculate_hand_value(&hand.cards) > 21 {
                                hand.status = HandStatus::Busted;
                            } else {
                                hand.status = HandStatus::Doubled;
                            }
                            should_advance = true;
                        }
                    }
                }
            }
            ActionResult::Split(bet_amount) => {
                let card_for_first = self.draw_card();
                let card_for_second = self.draw_card();

                if let Some(player) = self.players.get_mut(self.turn_index) {
                    player.chips -= bet_amount;

                    let index = player.active_hand_index;

                    if let Some(hand) = player.hands.get_mut(index) {
                        if let Some(split_card) = hand.cards.pop() {
                            if let Some(c) = card_for_first {
                                hand.cards.push(c);
                            }

                            let mut new_hand_cards = vec![split_card];
                            if let Some(c) = card_for_second {
                                new_hand_cards.push(c);
                            }

                            let new_hand = Hand {
                                cards: new_hand_cards,
                                bet: bet_amount,
                                status: HandStatus::Playing,
                            };
                            player.hands.insert(index + 1, new_hand);
                        }
                    }
                }
            }
            ActionResult::None => {}
        }

        if should_advance {
            self.advance_turn();
        } else {
            self.broadcast_state();
        }
    }

    fn advance_turn(&mut self) {
        if let Some(player) = self.players.get_mut(self.turn_index) {
            // Only advance hand index if this player is actually playing
            if player.status == PlayerStatus::Playing
                && player.active_hand_index + 1 < player.hands.len()
            {
                player.active_hand_index += 1;
                self.broadcast_state();
                return;
            }
        }

        loop {
            self.turn_index += 1;

            if self.turn_index >= self.players.len() {
                self.play_dealer_turn();
                return;
            }

            // Skip players who are not Playing (e.g. Sitting or Pending) or Disconnected
            if let Some(player) = self.players.get(self.turn_index) {
                if player.status == PlayerStatus::Playing && player.is_connected {
                    // Also ensure the player has a playable hand (skip if Blackjack)
                    if let Some(hand) = player.hands.get(player.active_hand_index) {
                        if hand.status == HandStatus::Playing {
                            break;
                        }
                    }
                }
            }
        }

        self.broadcast_state();
    }

    fn play_dealer_turn(&mut self) {
        self.phase = GamePhase::DealerTurn;

        while calculate_hand_value(&self.dealer_hand) < 17 {
            if let Some(card) = self.draw_card() {
                self.dealer_hand.push(card);
            } else {
                break;
            }
        }

        self.resolve_bets();
    }

    fn resolve_bets(&mut self) {
        let dealer_value = calculate_hand_value(&self.dealer_hand);
        let dealer_is_blackjack = dealer_value == 21 && self.dealer_hand.len() == 2;

        for player in self.players.iter_mut() {
            if player.status != PlayerStatus::Playing {
                continue;
            }

            for hand in player.hands.iter_mut() {
                let hand_value = calculate_hand_value(&hand.cards);

                if hand.status == HandStatus::Busted {
                    // Player loses bet
                    hand.status = HandStatus::Lost;
                } else if hand.status == HandStatus::Blackjack {
                    if dealer_is_blackjack {
                        player.chips += hand.bet; // Push, return bet
                        hand.status = HandStatus::Push;
                    } else {
                        player.chips += (hand.bet as f32 * 2.5) as u32;
                        hand.status = HandStatus::Won;
                    }
                } else if dealer_is_blackjack {
                    // Player does not have Blackjack, but Dealer does -> Player loses
                    hand.status = HandStatus::Lost;
                } else if dealer_value > 21 || hand_value > dealer_value {
                    player.chips += hand.bet * 2;
                    hand.status = HandStatus::Won;
                } else if hand_value == dealer_value {
                    player.chips += hand.bet; // Push, return bet
                    hand.status = HandStatus::Push;
                } else {
                    hand.status = HandStatus::Lost;
                }

                // hand.bet = 0; // Keep bet amount for display during Payout
            }

            player.status = PlayerStatus::Sitting;
            // player.hands.clear(); // Keep hands for display during Payout
        }

        // self.dealer_hand.clear(); // Keep dealer hand for display during Payout
        self.phase = GamePhase::Payout;
        self.broadcast_state();
    }

    fn get_current_player(&self) -> Option<&Player> {
        self.players.get(self.turn_index)
    }

    fn start_betting_phase(&mut self) {
        self.phase = GamePhase::Betting;
        self.dealer_hand.clear();

        // Spectators stay Spectating.
        // Playing players move to Sitting (waiting to bet).
        // Pending stay Pending.
        for player in self.players.iter_mut() {
            player.hands.clear();
            if player.status == PlayerStatus::Playing
                || player.status == PlayerStatus::Sitting
                || player.status == PlayerStatus::Spectating
            {
                // Everyone (except pending) becomes eligible to bet = Sitting
                // The term Spectating in this codebase has been used loosely.
                // Let's say: if you are in the room, you are Sitting and can bet.
                // Unless you are Pending.
                if player.status != PlayerStatus::PendingApproval {
                    player.status = PlayerStatus::Sitting;
                }
            }
        }
        self.broadcast_state();
    }

    fn start_action_phase(&mut self) {
        // Only consider players who have bet as active for this round.
        // Players who are 'Sitting' (did not bet) are skipped.
        self.phase = GamePhase::Playing;
        self.turn_index = 0;

        // Count how many players are actually playing
        let players_playing = self
            .players
            .iter()
            .filter(|p| p.status == PlayerStatus::Playing)
            .count();
        if players_playing == 0 {
            self.broadcast(ServerMessage::Error {
                msg: "Cannot start round with no active bets.".to_string(),
            });
            self.phase = GamePhase::Betting; // Revert
            return;
        }

        // Deal cards only to Playing players
        for _ in 0..2 {
            for i in 0..self.players.len() {
                if self.players[i].status == PlayerStatus::Playing {
                    if let Some(card) = self.draw_card() {
                        if let Some(player) = self.players.get_mut(i) {
                            if let Some(hand) = player.hands.get_mut(0) {
                                hand.cards.push(card);
                            }
                        }
                    }
                }
            }

            if let Some(card) = self.draw_card() {
                self.dealer_hand.push(card);
            }
        }

        // Check for Blackjacks immediately after deal
        for player in self.players.iter_mut() {
            if player.status == PlayerStatus::Playing {
                if let Some(hand) = player.hands.get_mut(0) {
                    if hand.cards.len() == 2 && calculate_hand_value(&hand.cards) == 21 {
                        hand.status = HandStatus::Blackjack;
                    }
                }
            }
        }

        // Set turn_index to the first active player who is connected and needs to act (no Blackjack)
        if let Some(pos) = self.players.iter().position(|p| {
            p.status == PlayerStatus::Playing
                && p.is_connected
                && p.hands
                    .get(0)
                    .map(|h| h.status == HandStatus::Playing)
                    .unwrap_or(false)
        }) {
            self.turn_index = pos;
        } else {
            // No connected players need to act (all Blackjacks or disconnected), dealer takes over
            self.play_dealer_turn();
            return;
        }

        self.broadcast_state();
    }

    fn draw_card(&mut self) -> Option<Card> {
        if self.deck.is_empty() {
            self.init_deck();
        }
        self.deck.pop()
    }

    fn broadcast_state(&self) {
        let mut sanitized_dealer = self.dealer_hand.clone();
        if self.phase == GamePhase::Playing && sanitized_dealer.len() >= 2 {
            sanitized_dealer.truncate(1);
        }

        let msg = ServerMessage::GameStateSnapshot {
            phase: self.phase.clone(),
            dealer_hand: sanitized_dealer,
            players: self.players.clone(),
            deck_remaining: self.deck.len(),
            current_turn_player_id: self.players.get(self.turn_index).map(|p| p.id),
            settings: self.settings.clone(),
        };

        self.broadcast(msg);
    }
}

enum ActionResult {
    None,
    Hit,
    Stand,
    Double(u32),
    Split(u32),
}
