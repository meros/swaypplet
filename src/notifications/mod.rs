//! The notification popups on screen: the stack of cards (`stack`), each
//! card's content (`card`) and menu (`menu`), and their auto-dismiss timers
//! (`timers`). What a notification is, the store that holds them and the
//! D-Bus server that receives them are services
//! (`crate::services::notifications`).

pub mod card;
pub mod markup;
mod menu;
pub mod stack;
mod timers;
