//! What one call costs, from provider-reported usage and configured list prices.
//!
//! `docs/AI_PROVIDER_SPEC.md` §7 says cost is **exposed, never inferred**: the prices a run is
//! charged at come from configuration, not from a table this crate ships, because a hard-coded
//! vendor price is wrong the day the vendor changes it and silently so. This module is the
//! arithmetic half only — [`Pricing`] turns the [`Usage`] a provider reported into a [`Cost`]. The
//! policy half (where the prices are recorded, and what to refuse when they are not) belongs to the
//! command that spends the money.
//!
//! # Units
//!
//! A list price is quoted in dollars per **million** tokens, and money is stored in **micro-USD**
//! (`1 µUSD = $0.000001`). Both are integers, so a price and a cost are exact: `$2.50` per million
//! tokens is `2_500_000`, and 1,200 tokens at that price is `3_000` `µUSD` (`$0.003000`). No `f64`
//! appears anywhere on the path, so two runs over the same usage produce the same total to the
//! micro-dollar.
//!
//! # What a token is charged at
//!
//! [`Usage::cached_input_tokens`] is a **subset** of [`Usage::input_tokens`]: every cached token is
//! also an input token. The adapters normalise their wire formats to that shape, because the three
//! vendors disagree about it on the wire. Cached tokens are charged at the cached-input price when
//! one is configured and at the input price otherwise; output tokens are charged at the output
//! price. Every component is rounded **up** to the micro-dollar, so a cost this module reports is
//! never below the arithmetic truth — the safe direction for a ceiling.
//!
//! A provider that reports a separate cache-*write* count is counted in `input_tokens` by its
//! adapter, and is therefore charged at the input price. A cache write is usually a little dearer
//! than a plain input token; this module does not invent a fourth price for it, and `Pricing`
//! documents the omission rather than pretending the count is exact.

use std::fmt;

use crate::Usage;

/// The denominator a list price is quoted per: one million tokens.
const PER_MILLION: u128 = 1_000_000;

/// Micro-dollars in one dollar, for rendering a [`Cost`].
const MICRO_PER_DOLLAR: u64 = 1_000_000;

/// A price in micro-USD per **million** tokens.
///
/// `Price::from_micro_usd(2_500_000)` is `$2.50` per million tokens. The unit is deliberately not
/// dollars per token: a per-token price is a fraction smaller than a micro-dollar for every model
/// this tool talks to, so it could not be stored exactly as an integer.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Price(u64);

impl Price {
    /// A price of `micro_usd` micro-dollars per million tokens.
    #[must_use]
    pub const fn from_micro_usd(micro_usd: u64) -> Self {
        Self(micro_usd)
    }

    /// The price in micro-USD per million tokens.
    #[must_use]
    pub const fn micro_usd(self) -> u64 {
        self.0
    }
}

/// A money amount in micro-USD.
///
/// [`Cost::ZERO`] is a real zero — a free provider with a configured price of zero — and is
/// deliberately distinct from an unknown cost, which is represented by `None` where a `Cost` is
/// optional. The two must never be conflated: a local server that reports no usage has an
/// *unknown* cost, not a free one.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Cost(u64);

impl Cost {
    /// No money at all.
    pub const ZERO: Self = Self(0);

    /// An amount of `micro_usd` micro-dollars.
    #[must_use]
    pub const fn from_micro_usd(micro_usd: u64) -> Self {
        Self(micro_usd)
    }

    /// The amount in micro-USD.
    #[must_use]
    pub const fn micro_usd(self) -> u64 {
        self.0
    }

    /// The amount in dollars, with exactly six decimal places.
    ///
    /// Fixed precision rather than trimmed zeros: a cost is compared against a configured ceiling by
    /// eye as well as by code, and `$0.004500` and `$0.0045` should not look like different values.
    #[must_use]
    pub fn usd(self) -> String {
        format!(
            "{}.{:06}",
            self.0 / MICRO_PER_DOLLAR,
            self.0 % MICRO_PER_DOLLAR
        )
    }
}

impl fmt::Display for Cost {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "${}", self.usd())
    }
}

/// The USD list prices one provider's model is charged at (`docs/AI_PROVIDER_SPEC.md` §7).
///
/// One profile pins one model, so one price set is enough. A local provider is configured with
/// [`Cost::ZERO`]-priced rates only if the developer wants its cost to read as free; leaving
/// prices out keeps the cost *unknown*, which is the honest answer for a server that may or may not
/// charge.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Pricing {
    input: Price,
    output: Price,
    cached_input: Option<Price>,
}

impl Pricing {
    /// Prices for input, output, and — when the provider discounts them — cached input tokens.
    ///
    /// `cached_input` may be `None`, in which case cached tokens are charged at the input price.
    #[must_use]
    pub const fn new(input: Price, output: Price, cached_input: Option<Price>) -> Self {
        Self {
            input,
            output,
            cached_input,
        }
    }

    /// The price of one million uncached input tokens.
    #[must_use]
    pub const fn input(self) -> Price {
        self.input
    }

    /// The price of one million output tokens.
    #[must_use]
    pub const fn output(self) -> Price {
        self.output
    }

    /// The price of one million cached input tokens, when one is configured.
    #[must_use]
    pub const fn cached_input(self) -> Option<Price> {
        self.cached_input
    }

    /// What the provider-reported `usage` costs, or `None` when usage is too incomplete to price.
    ///
    /// A total requires both an input and an output count: a sum of one half is not a total, and
    /// reporting it as one would understate the spend. A missing cached count is not incomplete —
    /// it means the provider reported no cache hits, which are charged at the input price.
    ///
    /// # Panics
    ///
    /// Never panics. Counts are `u32` and prices are `u64`, and the product is over `u128` before it
    /// is saturated into a `u64` amount, so no input can overflow the arithmetic.
    #[must_use]
    pub fn cost(self, usage: &Usage) -> Option<Cost> {
        let input = u64::from(usage.input_tokens?);
        let output = u64::from(usage.output_tokens?);
        let cached = u64::from(usage.cached_input_tokens.unwrap_or(0)).min(input);
        let uncached = input - cached;
        let cached_price = self.cached_input.unwrap_or(self.input);
        let total = charge(uncached, self.input)
            .saturating_add(charge(cached, cached_price))
            .saturating_add(charge(output, self.output));
        Some(Cost(saturate(total)))
    }

    /// The worst case for a request that has not been sent: `input_tokens` at the input price and
    /// `output_tokens` at the output price.
    ///
    /// No cache discount is applied, because a projection is made before the provider has said
    /// whether anything was cached; charging the undiscounted price is the conservative direction
    /// for a ceiling. The token counts are the caller's estimate, so the result is an estimate too
    /// and is labelled as one wherever it is shown (`docs/AI_PROVIDER_SPEC.md` §7).
    ///
    /// # Panics
    ///
    /// Never panics; the arithmetic is the same saturating `u128` calculation [`Self::cost`] uses.
    #[must_use]
    pub fn projected(self, input_tokens: u64, output_tokens: u64) -> Cost {
        let total =
            charge(input_tokens, self.input).saturating_add(charge(output_tokens, self.output));
        Cost(saturate(total))
    }
}

/// `tokens × price`, in micro-USD, rounded up to the next micro-dollar.
fn charge(tokens: u64, price: Price) -> u128 {
    (u128::from(tokens) * u128::from(price.0)).div_ceil(PER_MILLION)
}

/// A saturating `u128` to `u64` conversion for a money amount.
fn saturate(value: u128) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::{Cost, Price, Pricing};
    use crate::Usage;

    /// `$2.50` in, `$10.00` out, `$1.25` cached: a round set with an exact expected total.
    fn pricing() -> Pricing {
        Pricing::new(
            Price::from_micro_usd(2_500_000),
            Price::from_micro_usd(10_000_000),
            Some(Price::from_micro_usd(1_250_000)),
        )
    }

    #[test]
    fn a_cost_charges_cached_tokens_at_the_cached_price_and_the_rest_at_input() {
        // 1,200 input of which 200 cached, and 180 output. Uncached 1,000 × $2.50/M = $0.0025;
        // cached 200 × $1.25/M = $0.00025; output 180 × $10/M = $0.0018. Total $0.00455.
        let usage = Usage {
            input_tokens: Some(1200),
            output_tokens: Some(180),
            cached_input_tokens: Some(200),
            estimated: false,
        };
        let cost = pricing().cost(&usage).expect("both halves were reported");
        assert_eq!(cost, Cost::from_micro_usd(4_550));
        assert_eq!(cost.usd(), "0.004550");
        assert_eq!(cost.to_string(), "$0.004550");
    }

    #[test]
    fn a_missing_cached_price_charges_cache_hits_at_the_input_price() {
        let usage = Usage {
            input_tokens: Some(1_000),
            output_tokens: Some(0),
            cached_input_tokens: Some(1_000),
            estimated: false,
        };
        let pricing = Pricing::new(
            Price::from_micro_usd(2_500_000),
            Price::from_micro_usd(10_000_000),
            None,
        );
        assert_eq!(
            pricing.cost(&usage),
            Some(Cost::from_micro_usd(2_500)),
            "a cache hit is not free just because no cache price was configured"
        );
    }

    #[test]
    fn a_cached_count_larger_than_input_never_makes_the_total_negative() {
        // A malformed server can report more cache hits than input tokens. Clamping means the
        // uncached half is zero rather than an underflow.
        let usage = Usage {
            input_tokens: Some(100),
            output_tokens: Some(1),
            cached_input_tokens: Some(900),
            estimated: false,
        };
        assert_eq!(
            pricing().cost(&usage),
            Some(Cost::from_micro_usd(135)),
            "100 cached tokens at $1.25/M plus 1 output at $10/M"
        );
    }

    #[test]
    fn an_incomplete_usage_has_no_cost_rather_than_a_partial_one() {
        let no_output = Usage {
            input_tokens: Some(100),
            output_tokens: None,
            cached_input_tokens: None,
            estimated: false,
        };
        let no_input = Usage {
            input_tokens: None,
            output_tokens: Some(100),
            cached_input_tokens: None,
            estimated: false,
        };
        assert_eq!(pricing().cost(&no_output), None);
        assert_eq!(pricing().cost(&no_input), None);
        assert_eq!(pricing().cost(&Usage::default()), None);
    }

    #[test]
    fn a_projection_rounds_up_and_applies_no_cache_discount() {
        // 1 token at $2.50/M is 0.0000025 dollars, rounded up to $0.000003.
        let pricing = Pricing::new(
            Price::from_micro_usd(2_500_000),
            Price::from_micro_usd(10_000_000),
            Some(Price::from_micro_usd(0)),
        );
        assert_eq!(
            pricing.projected(1, 1),
            Cost::from_micro_usd(13),
            "3 µUSD in and 10 µUSD out, each rounded up"
        );
        assert_eq!(pricing.projected(0, 0), Cost::ZERO);
    }

    #[test]
    fn a_zero_price_is_a_real_zero() {
        let free = Pricing::new(Price::from_micro_usd(0), Price::from_micro_usd(0), None);
        let usage = Usage::reported(1_000_000, 1_000_000);
        assert_eq!(free.cost(&usage), Some(Cost::ZERO));
        assert_eq!(Cost::ZERO.usd(), "0.000000");
    }

    #[test]
    fn the_amount_is_rendered_with_six_fixed_decimals() {
        assert_eq!(Cost::from_micro_usd(1).usd(), "0.000001");
        assert_eq!(Cost::from_micro_usd(1_234_567).usd(), "1.234567");
        assert_eq!(Cost::from_micro_usd(1_000_000).usd(), "1.000000");
    }

    #[test]
    fn the_price_accessors_report_what_was_configured() {
        let pricing = pricing();
        assert_eq!(pricing.input().micro_usd(), 2_500_000);
        assert_eq!(pricing.output().micro_usd(), 10_000_000);
        assert_eq!(
            pricing.cached_input().map(Price::micro_usd),
            Some(1_250_000)
        );
    }
}
