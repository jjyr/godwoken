//! Godwoken mem pool
//! MemPool keeps l2transactions & withdrawal requests in an order.
//! MemPool only do basic verification on l2transactions & withdrawal requests,
//! the block producer need to verify the fully verification itself.

pub mod pool;
