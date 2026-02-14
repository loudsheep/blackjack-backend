use crate::game::types::*;
use crate::messages::{ActionType, ClientMessage, ServerMessage};
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

    // Channels
    receiver: mpsc::Receiver<(Uuid, ClientMessage)>, // We need to know WHO sent the msg
    sender: broadcast::Sender<ServerMessage>,
}

impl GameActor {
    pub fn new(
        game_id: String,
        settings: GameSettings,
        receiver: mpsc::Receiver<(Uuid, ClientMessage)>,
        sender: broadcast::Sender<ServerMessage>,
    ) -> Self {
        let mut actor = Self {
            game_id,
            settings,
            phase: GamePhase::Lobby,
            deck: Vec::new(),
            players: Vec::new(),
            dealer_hand: Vec::new(),
            turn_index: 0,
            receiver,
            sender,
        };
        actor.init_deck();
        actor
    }

    pub async fn run(&mut self) {
        while let Some((player_id, msg)) = self.receiver.recv().await {
            match msg {
                ClientMessage::JoinGame { username } => {
                    self.handle_join(player_id, username);
                }

                ClientMessage::PlaceBet { amount } => {
                    self.handle_bet(player_id, amount);
                }

                ClientMessage::GameAction { action_type } => {
                    self.handle_action(player_id, action_type);
                }

                ClientMessage::ApprovePlayer { player_id: target_id } => {
                    self.handle_approve(player_id, target_id);
                }

                ClientMessage::KickPlayer { player_id: target_id } => {
                    self.handle_kick(player_id, target_id);
                }

                ClientMessage::UpdateSettings { settings } => {
                    self.handle_update_settings(player_id, settings);
                }

                ClientMessage::AdminUpdateBalance { target_id, change_chips } => {
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
            }
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

    fn handle_join(&mut self, player_id: Uuid, username: String) {
        if self.players.iter().any(|p| p.id == player_id) {
            return;
        }

        if self.players.len() >= self.settings.max_players {
            let _ = self.sender.send(ServerMessage::Error {
                msg: "Game is full".to_string(),
            });
            // Ideally we would send this only to the connecting player, but broadcast sends to all.
            // A better architecture would allow unicast. For now, we broadcast error, clients should handle it.
            // However, this might spam others. 
            // Since we can't unicast easily with this broadcast channel, we might just return and let the client timeout or stay in limbo?
            // Or we check max players at connection time in ws.rs?
            // ws.rs doesn't check max players before upgrading.
            return;
        }

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
            let _ = self.sender.send(msg); // Admins should listen for this
        }

        self.players.push(Player {
            id: player_id,
            name: username,
            chips: self.settings.initial_chips,
            hands: vec![],
            active_hand_index: 0,
            status: status.clone(),
            is_admin: is_first,
        });

        // Broadcast JoinedLobby - clients should check if `your_id` matches theirs
        let _ = self.sender.send(ServerMessage::JoinedLobby {
            game_id: self.game_id.clone(),
            your_id: player_id,
            is_admin: is_first,
        });

        self.broadcast_state();
    }

    fn handle_approve(&mut self, admin_id: Uuid, target_id: Uuid) {
        if !self.is_admin(admin_id) { return; }
        
        if let Some(player) = self.players.iter_mut().find(|p| p.id == target_id) {
            if player.status == PlayerStatus::PendingApproval {
                player.status = PlayerStatus::Spectating;
                self.broadcast_state();
            }
        }
    }

    fn handle_kick(&mut self, admin_id: Uuid, target_id: Uuid) {
        if !self.is_admin(admin_id) { return; }

        // Remove player
        if let Some(pos) = self.players.iter().position(|p| p.id == target_id) {
            self.players.remove(pos);
            self.broadcast_state();
        }
    }

    fn handle_update_settings(&mut self, admin_id: Uuid, settings: GameSettings) {
        if !self.is_admin(admin_id) { return; }
        self.settings = settings;
    }

    fn handle_admin_update_balance(&mut self, admin_id: Uuid, target_id: Uuid, change_chips: i32) {
        if !self.is_admin(admin_id) { return; }

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
        if !self.settings.chat_enabled { return; }

        let sender_name = self.players.iter().find(|p| p.id == player_id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "Unknown".to_string());

        let _ = self.sender.send(ServerMessage::ChatBroadcast {
            from: sender_name,
            msg,
        });
    }

    fn handle_start_game(&mut self, player_id: Uuid) {
        if !self.is_admin(player_id) { return; }
        if self.phase == GamePhase::Lobby {
            self.start_betting_phase();
        }
    }

    fn handle_next_round(&mut self, player_id: Uuid) {
        if !self.is_admin(player_id) { return; }
        if self.phase == GamePhase::Payout {
            self.start_betting_phase();
        }
    }


    fn handle_bet(&mut self, player_id: Uuid, amount: u32) {
        if self.phase != GamePhase::Betting {
            return;
        }

        let mut status_changed = false;

        if let Some(player) = self.players.iter_mut().find(|p| p.id == player_id) {
            // Pending players cannot bet
            if player.status == PlayerStatus::PendingApproval {
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
            }
        }

        if !status_changed { return; }

        let all_bets_placed = self.players.iter().all(|p| {
            p.status == PlayerStatus::Spectating
                || p.status == PlayerStatus::PendingApproval
                || (p.status == PlayerStatus::Playing && !p.hands.is_empty())
        });

        if all_bets_placed {
            self.start_action_phase();
        } else {
            self.broadcast_state();
        }
    }

    fn handle_action(&mut self, player_id: Uuid, action: ActionType) {
        if self.phase != GamePhase::Playing {
            return;
        }

        let is_turn = self
            .get_current_player()
            .map(|p| p.id == player_id)
            .unwrap_or(false);

        if !is_turn {
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
                        }
                    }
                    ActionType::Split => {
                        if hand.cards.len() == 2 && player.chips >= hand.bet 
                            && hand.cards[0].rank == hand.cards[1].rank {
                            action_result = ActionResult::Split(hand.bet);
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
            if player.active_hand_index + 1 < player.hands.len() {
                player.active_hand_index += 1;
                self.broadcast_state();
                return;
            }
        }

        self.turn_index += 1;

        if self.turn_index >= self.players.len() {
            self.play_dealer_turn();
        } else {
            self.broadcast_state();
        }
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

        for player in self.players.iter_mut() {
            if player.status != PlayerStatus::Playing {
                continue;
            }

            for hand in player.hands.iter_mut() {
                let hand_value = calculate_hand_value(&hand.cards);

                if hand.status == HandStatus::Busted {
                    // Player loses bet
                } else if hand.status == HandStatus::Blackjack && dealer_value != 21 {
                    player.chips += (hand.bet as f32 * 2.5) as u32;
                } else if dealer_value > 21 || hand_value > dealer_value {
                    player.chips += hand.bet * 2;
                } else if hand_value == dealer_value {
                    player.chips += hand.bet; // Push, return bet
                }

                hand.bet = 0;
            }

            player.status = PlayerStatus::Sitting;
            player.hands.clear();
        }

        self.dealer_hand.clear();
        self.phase = GamePhase::Payout;
        self.broadcast_state();
    }

    fn get_current_player(&self) -> Option<&Player> {
        self.players.get(self.turn_index)
    }

    fn start_betting_phase(&mut self) {
        self.phase = GamePhase::Betting;
        for player in self.players.iter_mut() {
            if player.status != PlayerStatus::Spectating && player.status != PlayerStatus::PendingApproval {
                player.status = PlayerStatus::Playing;
            }
        }
        self.broadcast_state();
    }

    fn start_action_phase(&mut self) {
        self.phase = GamePhase::Playing;
        self.turn_index = 0;

        for _ in 0..2 {
            for i in 0..self.players.len() {
                let needs_card = self.players[i].status == PlayerStatus::Playing;

                if needs_card {
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
        };

        let _ = self.sender.send(msg);
    }
}

enum ActionResult {
    None,
    Hit,
    Stand,
    Double(u32),
    Split(u32),
}