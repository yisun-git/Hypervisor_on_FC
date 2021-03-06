// Copyright 2018 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#![deny(missing_docs)]
//! # Rate Limiter
//!
//! Provides a rate limiter written in Rust useful for IO operations that need to
//! be throttled.
//!
//! ## Behavior
//!
//! The rate limiter starts off as 'unblocked' with two token buckets configured
//! with the values passed in the `RateLimiter::new()` constructor.
//! All subsequent accounting is done independently for each token bucket based
//! on the `TokenType` used. If any of the buckets runs out of budget, the limiter
//! goes in the 'blocked' state. At this point an internal timer is set up which
//! will later 'wake up' the user in order to retry sending data. The 'wake up'
//! notification will be dispatched as an event on the FD provided by the `AsRawFD`
//! trait implementation.
//!
//! The contract is that the user shall also call the `event_handler()` method on
//! receipt of such an event.
//!
//! The token buckets are replenished every time a `consume()` is called, before
//! actually trying to consume the requested amount of tokens. The amount of tokens
//! replenished is automatically calculated to respect the `complete_refill_time`
//! configuration parameter provided by the user. The token buckets will never
//! replenish above their respective `size`.
//!
//! Each token bucket can start off with a `one_time_burst` initial extra capacity
//! on top of their `size`. This initial extra credit does not replenish and
//! can be used for an initial burst of data.
//!
//! The granularity for 'wake up' events when the rate limiter is blocked is
//! currently hardcoded to `100 milliseconds`.
//!
//! ## Limitations
//!
//! This rate limiter implementation relies on the *Linux kernel's timerfd* so its
//! usage is limited to Linux systems.
//!
//! Another particularity of this implementation is that it is not self-driving.
//! It is meant to be used in an external event loop and thus implements the `AsRawFd`
//! trait and provides an *event-handler* as part of its API. This *event-handler*
//! needs to be called by the user on every event on the rate limiter's `AsRawFd` FD.
//!

extern crate serde;
extern crate time;
extern crate timerfd;
#[macro_use]
extern crate serde_derive;

#[macro_use]
extern crate logger;

use std::os::unix::io::{AsRawFd, RawFd};
use std::time::Duration;
use std::{fmt, io};
use timerfd::{ClockId, SetTimeFlags, TimerFd, TimerState};

#[derive(Debug)]
/// Describes the errors that may occur while handling rate limiter events.
pub enum Error {
    /// The event handler was called spuriously.
    SpuriousRateLimiterEvent(&'static str),
}

// Interval at which the refill timer will run when limiter is at capacity.
const REFILL_TIMER_INTERVAL_MS: u64 = 100;
const TIMER_REFILL_STATE: TimerState =
    TimerState::Oneshot(Duration::from_millis(REFILL_TIMER_INTERVAL_MS));

const NANOSEC_IN_ONE_MILLISEC: u64 = 1_000_000;

// Euclid's two-thousand-year-old algorithm for finding the greatest common divisor.
fn gcd(x: u64, y: u64) -> u64 {
    let mut x = x;
    let mut y = y;
    while y != 0 {
        let t = y;
        y = x % y;
        x = t;
    }
    x
}

/// TokenBucket provides a lower level interface to rate limiting with a
/// configurable capacity, refill-rate and initial burst.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TokenBucket {
    // Bucket defining traits.
    size: u64,
    // Initial burst size (number of free initial tokens, that can be consumed at no cost)
    one_time_burst: Option<u64>,
    // Complete refill time in milliseconds.
    refill_time: u64,

    // Internal state descriptors.
    #[serde(skip)]
    budget: u64,
    #[serde(skip)]
    last_update: u64,

    // Fields used for pre-processing optimizations.
    #[serde(skip)]
    processed_capacity: u64,
    #[serde(skip)]
    processed_refill_time: u64,
}

impl TokenBucket {
    /// Creates a TokenBucket of `size` total capacity that takes `complete_refill_time_ms`
    /// milliseconds to go from zero tokens to total capacity. The `one_time_burst` is initial
    /// extra credit on top of total capacity, that does not replenish and which can be used
    /// for an initial burst of data.
    pub fn new(size: u64, one_time_burst: Option<u64>, complete_refill_time_ms: u64) -> Self {
        // Formula for computing current refill amount:
        // refill_token_count = (delta_time * size) / (complete_refill_time_ms * 1_000_000)
        // In order to avoid overflows, simplify the fractions by computing greatest common divisor.

        // Get the greatest common factor between `size` and `complete_refill_time_ms`.
        let common_factor = gcd(size, complete_refill_time_ms);
        // The division will be exact since `common_factor` is a factor of `size`.
        let mut processed_capacity: u64 = size / common_factor;
        // The division will be exact since `common_factor` is a factor of `complete_refill_time_ms`.
        let mut processed_refill_time: u64 = complete_refill_time_ms / common_factor;

        // Get the gcd between `processed_capacity` and `NANOSEC_IN_ONE_MILLISEC`
        // which is 1_000_000 (see formula from above).
        let common_factor = gcd(processed_capacity, NANOSEC_IN_ONE_MILLISEC);
        // Reduce the capacity factor even further.
        processed_capacity /= common_factor;
        // `processed_refill_time` was ms; turn to nanoseconds and reduce by `common_factor`.
        processed_refill_time *= NANOSEC_IN_ONE_MILLISEC / common_factor;

        TokenBucket {
            size,
            one_time_burst,
            refill_time: complete_refill_time_ms,
            // Start off full.
            budget: size,
            // Last updated is now.
            last_update: time::precise_time_ns(),
            processed_capacity,
            processed_refill_time,
        }
    }

    /// Attempts to consume `tokens` from the bucket and returns whether the action succeeded.
    // TODO (Issue #259): handle cases where a single request is larger than the full capacity
    // for such cases we need to support partial fulfilment of requests
    pub fn reduce(&mut self, mut tokens: u64) -> bool {
        // First things first: consume the one-time-burst budget.
        if let Some(otb) = self.one_time_burst.as_mut() {
            if *otb > 0 {
                // We still have burst budget for *all* tokens requests.
                if *otb >= tokens {
                    *otb -= tokens;
                    self.last_update = time::precise_time_ns();
                    // No need to continue to the refill process, we still have burst budget to consume from.
                    return true;
                } else {
                    // We still have burst budget for *some* of the tokens requests.
                    // The tokens left unfulfilled will be consumed from current `self.budget`.
                    tokens -= *otb;
                    *otb = 0;
                }
            }
        }
        // Compute time passed since last refill/update.
        let now = time::precise_time_ns();
        let time_delta = now - self.last_update;
        self.last_update = now;

        // At each 'time_delta' nanoseconds the bucket should refill with:
        // refill_amount = (time_delta * size) / (complete_refill_time_ms * 1_000_000)
        // `processed_capacity` and `processed_refill_time` are the result of simplifying above
        // fraction formula with their greatest-common-factor.
        self.budget += (time_delta * self.processed_capacity) / self.processed_refill_time;

        if self.budget >= self.size {
            self.budget = self.size;
        }

        if tokens > self.budget {
            // TODO (Issue #259) remove this block when issue is resolved
            if tokens > self.size {
                error!(
                    "Trying to consume more tokens {} than the total capacity {}",
                    tokens, self.size
                );
                // best effort rate-limiting, this is a dirty workaround for Issue #259
                if self.budget == self.size {
                    self.budget = 0;
                    return true;
                }
            }
            // If not enough tokens consume() fails, return false.
            return false;
        }

        self.budget -= tokens;
        true
    }

    /// "Manually" adds tokens to bucket.
    pub fn replenish(&mut self, tokens: u64) {
        // This means we are still during the burst interval.
        // Of course there is a very small chance  that the last reduce() also used up burst
        // budget which should now be replenished, but for performance and code-complexity
        // reasons we're just gonna let that slide since it's practically inconsequential.
        if let Some(otb) = self.one_time_burst.as_mut() {
            if *otb > 0 {
                *otb += tokens;
                return;
            }
        }
        self.budget = std::cmp::min(self.budget + tokens, self.size);
    }

    /// Returns the capacity of the token bucket.
    pub fn capacity(&self) -> u64 {
        self.size
    }

    /// Returns the remaining one time burst budget.
    pub fn one_time_burst(&self) -> u64 {
        self.one_time_burst.unwrap_or(0)
    }

    /// Returns the time in milliseconds required to to completely fill the bucket.
    pub fn refill_time_ms(&self) -> u64 {
        self.refill_time
    }

    /// Returns the current budget (one time burst allowance notwithstanding).
    pub fn budget(&self) -> u64 {
        self.budget
    }
}

/// Enum that describes the type of token used.
pub enum TokenType {
    /// Token type used for bandwidth limiting.
    Bytes,
    /// Token type used for operations/second limiting.
    Ops,
}

/// Rate Limiter that works on both bandwidth and ops/s limiting.
///
/// Bandwidth (bytes/s) and ops/s limiting can be used at the same time or individually.
///
/// Implementation uses a single timer through TimerFd to refresh either or
/// both token buckets.
///
/// Its internal buckets are 'passively' replenished as they're being used (as
/// part of `consume()` operations).
/// A timer is enabled and used to 'actively' replenish the token buckets when
/// limiting is in effect and `consume()` operations are disabled.
///
/// RateLimiters will generate events on the FDs provided by their `AsRawFd` trait
/// implementation. These events are meant to be consumed by the user of this struct.
/// On each such event, the user must call the `event_handler()` method.
pub struct RateLimiter {
    bandwidth: Option<TokenBucket>,
    ops: Option<TokenBucket>,

    timer_fd: TimerFd,
    // Internal flag that quickly determines timer state.
    timer_active: bool,
}

impl PartialEq for RateLimiter {
    fn eq(&self, other: &RateLimiter) -> bool {
        self.bandwidth == other.bandwidth && self.ops == other.ops
    }
}

impl fmt::Debug for RateLimiter {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "RateLimiter {{ bandwidth: {:?}, ops: {:?} }}",
            self.bandwidth, self.ops
        )
    }
}

impl RateLimiter {
    // description
    fn make_bucket(
        total_capacity: u64,
        one_time_burst: Option<u64>,
        complete_refill_time_ms: u64,
    ) -> Option<TokenBucket> {
        // If either token bucket capacity or refill time is 0, disable limiting.
        if total_capacity != 0 && complete_refill_time_ms != 0 {
            Some(TokenBucket::new(
                total_capacity,
                one_time_burst,
                complete_refill_time_ms,
            ))
        } else {
            None
        }
    }

    /// Creates a new Rate Limiter that can limit on both bytes/s and ops/s.
    ///
    /// # Arguments
    ///
    /// * `bytes_total_capacity` - the total capacity of the `TokenType::Bytes` token bucket.
    /// * `bytes_one_time_burst` - initial extra credit on top of `bytes_total_capacity`,
    /// that does not replenish and which can be used for an initial burst of data.
    /// * `bytes_complete_refill_time_ms` - number of milliseconds for the `TokenType::Bytes`
    /// token bucket to go from zero Bytes to `bytes_total_capacity` Bytes.
    /// * `ops_total_capacity` - the total capacity of the `TokenType::Ops` token bucket.
    /// * `ops_one_time_burst` - initial extra credit on top of `ops_total_capacity`,
    /// that does not replenish and which can be used for an initial burst of data.
    /// * `ops_complete_refill_time_ms` - number of milliseconds for the `TokenType::Ops` token
    /// bucket to go from zero Ops to `ops_total_capacity` Ops.
    ///
    /// If either bytes/ops *size* or *refill_time* are **zero**, the limiter
    /// is **disabled** for that respective token type.
    ///
    /// # Errors
    ///
    /// If the timerfd creation fails, an error is returned.
    pub fn new(
        bytes_total_capacity: u64,
        bytes_one_time_burst: Option<u64>,
        bytes_complete_refill_time_ms: u64,
        ops_total_capacity: u64,
        ops_one_time_burst: Option<u64>,
        ops_complete_refill_time_ms: u64,
    ) -> io::Result<Self> {
        let bytes_token_bucket = Self::make_bucket(
            bytes_total_capacity,
            bytes_one_time_burst,
            bytes_complete_refill_time_ms,
        );

        let ops_token_bucket = Self::make_bucket(
            ops_total_capacity,
            ops_one_time_burst,
            ops_complete_refill_time_ms,
        );

        // We'll need a timer_fd, even if our current config effectively disables rate limiting,
        // because `Self::update_buckets()` might re-enable it later, and we might be
        // seccomp-blocked from creating the timer_fd at that time.
        let timer_fd = TimerFd::new_custom(ClockId::Monotonic, true, true)?;

        Ok(RateLimiter {
            bandwidth: bytes_token_bucket,
            ops: ops_token_bucket,
            timer_fd,
            timer_active: false,
        })
    }

    /// Attempts to consume tokens and returns whether that is possible.
    ///
    /// If rate limiting is disabled on provided `token_type`, this function will always succeed.
    pub fn consume(&mut self, tokens: u64, token_type: TokenType) -> bool {
        // Identify the required token bucket.
        let token_bucket = match token_type {
            TokenType::Bytes => self.bandwidth.as_mut(),
            TokenType::Ops => self.ops.as_mut(),
        };
        // Try to consume from the token bucket.
        let success = match token_bucket {
            Some(bucket) => bucket.reduce(tokens),
            // If bucket is not present rate limiting is disabled on token type,
            // consume() will always succeed.
            None => true,
        };
        // When we report budget is over, there will be no further calls here,
        // register a timer to replenish the bucket and resume processing;
        // make sure there is only one running timer for this limiter.
        if !success && !self.timer_active {
            // Register the timer; don't care about its previous state
            // safe to unwrap: timer is definitely Some() since we have a bucket.
            self.timer_fd
                .set_state(TIMER_REFILL_STATE, SetTimeFlags::Default);
            self.timer_active = true;
        }
        success
    }

    /// Adds tokens of `token_type` to their respective bucket.
    ///
    /// Can be used to *manually* add tokens to a bucket. Useful for reverting a
    /// `consume()` if needed.
    pub fn manual_replenish(&mut self, tokens: u64, token_type: TokenType) {
        // Identify the required token bucket.
        let token_bucket = match token_type {
            TokenType::Bytes => self.bandwidth.as_mut(),
            TokenType::Ops => self.ops.as_mut(),
        };
        // Add tokens to the token bucket.
        if let Some(bucket) = token_bucket {
            bucket.replenish(tokens);
        }
    }

    /// Returns whether this rate limiter is blocked.
    ///
    /// The limiter 'blocks' when a `consume()` operation fails because there was not enough
    /// budget for it.
    /// An event will be generated on the exported FD when the limiter 'unblocks'.
    pub fn is_blocked(&self) -> bool {
        self.timer_active
    }

    /// This function needs to be called every time there is an event on the
    /// FD provided by this object's `AsRawFd` trait implementation.
    ///
    /// # Errors
    ///
    /// If the rate limiter is disabled or is not blocked, an error is returned.
    pub fn event_handler(&mut self) -> Result<(), Error> {
        match self.timer_fd.read() {
            0 => Err(Error::SpuriousRateLimiterEvent(
                "Rate limiter event handler called without a present timer",
            )),
            _ => {
                self.timer_active = false;
                Ok(())
            }
        }
    }

    /// Updates the parameters of the token buckets associated with this RateLimiter.
    // TODO: Pls note that, right now, the buckets become full after being updated.
    pub fn update_buckets(&mut self, bytes: Option<TokenBucket>, ops: Option<TokenBucket>) {
        // TODO: We should reconcile the create and update paths, such that they use the same data
        // format. Currently, the TokenBucket config data is used for create, but the live
        // TokenBucket objects are used for update.
        // We have to call make_bucket instead of directly assigning the bytes and/or ops
        // because the RateLimiter validates the TokenBucket config data (e.g. it nullifies
        // an unusable bucket with size 0). This is needed, because passing a 0-sized bucket is
        // the only method the user has to disable rate limiting. I.e. if the user passes `null`
        // as the token bucket config, the old config is left unchanged.

        if let Some(b) = bytes {
            self.bandwidth = Self::make_bucket(b.size, b.one_time_burst, b.refill_time);
        }

        if let Some(b) = ops {
            self.ops = Self::make_bucket(b.size, b.one_time_burst, b.refill_time);
        }
    }

    /// Returns an immutable view of the inner bandwidth token bucket.
    pub fn bandwidth(&self) -> Option<&TokenBucket> {
        self.bandwidth.as_ref()
    }

    /// Returns an immutable view of the inner ops token bucket.
    pub fn ops(&self) -> Option<&TokenBucket> {
        self.ops.as_ref()
    }
}

impl AsRawFd for RateLimiter {
    /// Provides a FD which needs to be monitored for POLLIN events.
    ///
    /// This object's `event_handler()` method must be called on such events.
    ///
    /// Will return a negative value if rate limiting is disabled on both
    /// token types.
    fn as_raw_fd(&self) -> RawFd {
        self.timer_fd.as_raw_fd()
    }
}

impl Default for RateLimiter {
    /// Default RateLimiter is a no-op limiter with infinite budget.
    fn default() -> Self {
        // Safe to unwrap since this will not attempt to create timer_fd.
        RateLimiter::new(0, None, 0, 0, None, 0).expect("Failed to build default RateLimiter")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    impl TokenBucket {
        // Resets the token bucket: budget set to max capacity and last-updated set to now.
        fn reset(&mut self) {
            self.budget = self.size;
            self.last_update = time::precise_time_ns();
        }

        fn get_last_update(&self) -> u64 {
            self.last_update
        }

        fn get_processed_capacity(&self) -> u64 {
            self.processed_capacity
        }

        fn get_processed_refill_time(&self) -> u64 {
            self.processed_refill_time
        }
    }

    impl RateLimiter {
        fn get_token_bucket(&self, token_type: TokenType) -> Option<&TokenBucket> {
            match token_type {
                TokenType::Bytes => self.bandwidth.as_ref(),
                TokenType::Ops => self.ops.as_ref(),
            }
        }
    }

    #[test]
    fn test_token_bucket_create() {
        let before = time::precise_time_ns();
        let tb = TokenBucket::new(1000, None, 1000);
        assert_eq!(tb.capacity(), 1000);
        assert_eq!(tb.budget(), 1000);
        assert!(tb.get_last_update() >= before);
        assert!(tb.get_last_update() <= time::precise_time_ns());
        assert_eq!(tb.get_processed_capacity(), 1);
        assert_eq!(tb.get_processed_refill_time(), 1_000_000);
    }

    #[test]
    fn test_token_bucket_preprocess() {
        let tb = TokenBucket::new(1000, None, 1000);
        assert_eq!(tb.get_processed_capacity(), 1);
        assert_eq!(tb.get_processed_refill_time(), NANOSEC_IN_ONE_MILLISEC);

        let thousand = 1000;
        let tb = TokenBucket::new(3 * 7 * 11 * 19 * thousand, None, 7 * 11 * 13 * 17);
        assert_eq!(tb.get_processed_capacity(), 3 * 19);
        assert_eq!(
            tb.get_processed_refill_time(),
            13 * 17 * (NANOSEC_IN_ONE_MILLISEC / thousand)
        );
    }

    #[test]
    fn test_token_bucket_reduce() {
        // token bucket with capacity 1000 and refill time of 1000 milliseconds
        // allowing rate of 1 token/ms.
        let capacity = 1000;
        let refill_ms = 1000;
        let mut tb = TokenBucket::new(capacity, None, refill_ms as u64);

        assert!(tb.reduce(123));
        assert_eq!(tb.budget(), capacity - 123);

        thread::sleep(Duration::from_millis(123));
        assert!(tb.reduce(1));
        assert_eq!(tb.budget(), capacity - 1);
        assert!(tb.reduce(100));
        assert!(!tb.reduce(capacity));

        // token bucket with capacity 1000 and refill time of 1000 milliseconds
        let mut tb = TokenBucket::new(1000, Some(1100), 1000);
        // safely assuming the thread can run these 3 commands in less than 500ms
        assert!(tb.reduce(1000));
        assert_eq!(tb.one_time_burst(), 100);
        assert!(tb.reduce(500));
        assert_eq!(tb.one_time_burst(), 0);
        assert!(tb.reduce(500));
        assert!(!tb.reduce(500));
        thread::sleep(Duration::from_millis(500));
        assert!(tb.reduce(500));

        let before = time::precise_time_ns();
        tb.reset();
        assert_eq!(tb.capacity(), 1000);
        assert_eq!(tb.budget(), 1000);
        assert!(tb.get_last_update() >= before);
        assert!(tb.get_last_update() <= time::precise_time_ns());
    }

    #[test]
    fn test_rate_limiter_default() {
        let mut l = RateLimiter::default();

        // limiter should not be blocked
        assert!(!l.is_blocked());
        // limiter should be disabled so consume(whatever) should work
        assert!(l.consume(u64::max_value(), TokenType::Ops));
        assert!(l.consume(u64::max_value(), TokenType::Bytes));
        // calling the handler without there having been an event should error
        assert!(l.event_handler().is_err());
        assert_eq!(
            format!("{:?}", l.event_handler().err().unwrap()),
            "SpuriousRateLimiterEvent(\
             \"Rate limiter event handler called without a present timer\")"
        );
    }

    #[test]
    fn test_rate_limiter_new() {
        let l = RateLimiter::new(1000, Some(1001), 1002, 1003, Some(1004), 1005).unwrap();

        let bw = l.bandwidth.unwrap();
        assert_eq!(bw.capacity(), 1000);
        assert_eq!(bw.one_time_burst(), 1001);
        assert_eq!(bw.refill_time_ms(), 1002);
        assert_eq!(bw.budget(), 1000);

        let ops = l.ops.unwrap();
        assert_eq!(ops.capacity(), 1003);
        assert_eq!(ops.one_time_burst(), 1004);
        assert_eq!(ops.refill_time_ms(), 1005);
        assert_eq!(ops.budget(), 1003);
    }

    #[test]
    fn test_rate_limiter_manual_replenish() {
        // rate limiter with limit of 1000 bytes/s and 1000 ops/s
        let mut l = RateLimiter::new(1000, None, 1000, 1000, None, 1000).unwrap();

        // consume 123 bytes
        assert!(l.consume(123, TokenType::Bytes));
        l.manual_replenish(23, TokenType::Bytes);
        {
            let bytes_tb = l.get_token_bucket(TokenType::Bytes).unwrap();
            assert_eq!(bytes_tb.budget(), 900);
        }
        // consume 123 ops
        assert!(l.consume(123, TokenType::Ops));
        l.manual_replenish(23, TokenType::Ops);
        {
            let bytes_tb = l.get_token_bucket(TokenType::Ops).unwrap();
            assert_eq!(bytes_tb.budget(), 900);
        }
    }

    #[test]
    fn test_rate_limiter_bandwidth() {
        // rate limiter with limit of 1000 bytes/s
        let mut l = RateLimiter::new(1000, None, 1000, 0, None, 0).unwrap();

        // limiter should not be blocked
        assert!(!l.is_blocked());
        // raw FD for this disabled should be valid
        assert!(l.as_raw_fd() > 0);

        // ops/s limiter should be disabled so consume(whatever) should work
        assert!(l.consume(u64::max_value(), TokenType::Ops));

        // do full 1000 bytes
        assert!(l.consume(1000, TokenType::Bytes));
        // try and fail on another 100
        assert!(!l.consume(100, TokenType::Bytes));
        // since consume failed, limiter should be blocked now
        assert!(l.is_blocked());
        // wait half the timer period
        thread::sleep(Duration::from_millis(REFILL_TIMER_INTERVAL_MS / 2));
        // limiter should still be blocked
        assert!(l.is_blocked());
        // wait the other half of the timer period
        thread::sleep(Duration::from_millis(REFILL_TIMER_INTERVAL_MS / 2));
        // the timer_fd should have an event on it by now
        assert!(l.event_handler().is_ok());
        // limiter should now be unblocked
        assert!(!l.is_blocked());
        // try and succeed on another 100 bytes this time
        assert!(l.consume(100, TokenType::Bytes));
    }

    #[test]
    fn test_rate_limiter_ops() {
        // rate limiter with limit of 1000 ops/s
        let mut l = RateLimiter::new(0, None, 0, 1000, None, 1000).unwrap();

        // limiter should not be blocked
        assert!(!l.is_blocked());
        // raw FD for this disabled should be valid
        assert!(l.as_raw_fd() > 0);

        // bytes/s limiter should be disabled so consume(whatever) should work
        assert!(l.consume(u64::max_value(), TokenType::Bytes));

        // do full 1000 ops
        assert!(l.consume(1000, TokenType::Ops));
        // try and fail on another 100
        assert!(!l.consume(100, TokenType::Ops));
        // since consume failed, limiter should be blocked now
        assert!(l.is_blocked());
        // wait half the timer period
        thread::sleep(Duration::from_millis(REFILL_TIMER_INTERVAL_MS / 2));
        // limiter should still be blocked
        assert!(l.is_blocked());
        // wait the other half of the timer period
        thread::sleep(Duration::from_millis(REFILL_TIMER_INTERVAL_MS / 2));
        // the timer_fd should have an event on it by now
        assert!(l.event_handler().is_ok());
        // limiter should now be unblocked
        assert!(!l.is_blocked());
        // try and succeed on another 100 ops this time
        assert!(l.consume(100, TokenType::Ops));
    }

    #[test]
    fn test_rate_limiter_full() {
        // rate limiter with limit of 1000 bytes/s and 1000 ops/s
        let mut l = RateLimiter::new(1000, None, 1000, 1000, None, 1000).unwrap();

        // limiter should not be blocked
        assert!(!l.is_blocked());
        // raw FD for this disabled should be valid
        assert!(l.as_raw_fd() > 0);

        // do full 1000 bytes
        assert!(l.consume(1000, TokenType::Ops));
        // do full 1000 bytes
        assert!(l.consume(1000, TokenType::Bytes));
        // try and fail on another 100 ops
        assert!(!l.consume(100, TokenType::Ops));
        // try and fail on another 100 bytes
        assert!(!l.consume(100, TokenType::Bytes));
        // since consume failed, limiter should be blocked now
        assert!(l.is_blocked());
        // wait half the timer period
        thread::sleep(Duration::from_millis(REFILL_TIMER_INTERVAL_MS / 2));
        // limiter should still be blocked
        assert!(l.is_blocked());
        // wait the other half of the timer period
        thread::sleep(Duration::from_millis(REFILL_TIMER_INTERVAL_MS / 2));
        // the timer_fd should have an event on it by now
        assert!(l.event_handler().is_ok());
        // limiter should now be unblocked
        assert!(!l.is_blocked());
        // try and succeed on another 100 ops this time
        assert!(l.consume(100, TokenType::Ops));
        // try and succeed on another 100 bytes this time
        assert!(l.consume(100, TokenType::Bytes));

        // TODO (Issue #259) enable this check when issue is resolved
        // fail with warning on consume() > size
        //assert!(!l.consume(u64::max_value(), TokenType::Bytes));
    }

    #[test]
    fn test_update_buckets() {
        let mut x = RateLimiter::new(1000, Some(2000), 1000, 10, Some(20), 1000).unwrap();

        let initial_bw = x.bandwidth.clone();
        let initial_ops = x.ops.clone();

        x.update_buckets(None, None);
        assert_eq!(x.bandwidth, initial_bw);
        assert_eq!(x.ops, initial_ops);

        let new_bw = TokenBucket::new(123, None, 57);
        let new_ops = TokenBucket::new(321, Some(12346), 89);
        x.update_buckets(Some(new_bw.clone()), Some(new_ops.clone()));

        // We have manually adjust the last_update field, because it changes when update_buckets()
        // constructs new buckets (and thus gets a different value for last_update). We do this so
        // it makes sense to test the following assertions.
        x.bandwidth.as_mut().unwrap().last_update = new_bw.last_update;
        x.ops.as_mut().unwrap().last_update = new_ops.last_update;

        assert_eq!(x.bandwidth, Some(new_bw));
        assert_eq!(x.ops, Some(new_ops));
    }

    #[test]
    fn test_rate_limiter_debug() {
        let l = RateLimiter::new(1, Some(2), 3, 4, Some(5), 6).unwrap();
        assert_eq!(
            format!("{:?}", l),
            format!(
                "RateLimiter {{ bandwidth: {:?}, ops: {:?} }}",
                l.bandwidth(),
                l.ops()
            ),
        );
    }
}
