/// Liquidity calculation module for DEX pools.
///
/// This module provides functionality to calculate total liquidity in USD
/// for liquidity pools, handling decimal conversions and validation.

use crate::error::AppError;

/// SOL has 9 decimals (1 SOL = 1,000,000,000 lamports)
const SOL_DECIMALS: u8 = 9;

/// Maximum reasonable liquidity threshold ($1 billion)
const MAX_REASONABLE_LIQUIDITY_USD: f64 = 1_000_000_000.0;

/// Calculate total liquidity in USD for a pool.
///
/// This function converts reserve amounts from their smallest units (lamports for SOL,
/// smallest token units for other tokens) to full units, multiplies by their respective
/// prices, and sums them to get the total liquidity in USD.
///
/// # Arguments
///
/// * `sol_reserves` - Amount of SOL in lamports (1 SOL = 10^9 lamports)
/// * `token_reserves` - Amount of tokens in smallest units
/// * `sol_price` - Price of SOL in USD
/// * `token_price` - Price of the token in USD
/// * `token_decimals` - Number of decimal places for the token (e.g., 6 for USDC, 9 for most tokens)
///
/// # Note on SOL Decimals
///
/// SOL always has 9 decimals (hardcoded as `SOL_DECIMALS` constant). This is a fixed
/// property of the Solana blockchain and never changes, so it doesn't need to be
/// parameterized. The `token_decimals` parameter is required because different SPL tokens
/// can have varying decimal places (e.g., USDC has 6, while most tokens have 9).
///
/// # Returns
///
/// * `Ok(f64)` - Total liquidity in USD, rounded to 2 decimal places
/// * `Err(AppError)` - If the calculation produces invalid results (NaN, Infinity, negative)
///
/// # Examples
///
/// ```
/// use datanalyzer::liquidity::calculate_liquidity_usd;
///
/// // Pool with 10 SOL and 1000 USDC (6 decimals)
/// let liquidity = calculate_liquidity_usd(
///     10_000_000_000,  // 10 SOL in lamports
///     1_000_000_000,   // 1000 USDC in smallest units (6 decimals)
///     100.0,           // SOL price = $100
///     1.0,             // USDC price = $1
///     6                // USDC has 6 decimals
/// ).unwrap();
///
/// // Total liquidity = (10 * 100) + (1000 * 1) = $2000
/// assert_eq!(liquidity, 2000.0);
/// ```
pub fn calculate_liquidity_usd(
    sol_reserves: u64,
    token_reserves: u64,
    sol_price: f64,
    token_price: f64,
    token_decimals: u8,
) -> Result<f64, AppError> {
    // Handle the case when both reserves are zero (valid state, empty pool)
    if sol_reserves == 0 && token_reserves == 0 {
        return Ok(0.0);
    }

    // Convert reserves from smallest units to full units
    let sol_amount = convert_to_full_units(sol_reserves, SOL_DECIMALS);
    let token_amount = convert_to_full_units(token_reserves, token_decimals);

    // Calculate USD values
    let sol_value_usd = sol_amount * sol_price;
    let token_value_usd = token_amount * token_price;

    // Sum to get total liquidity
    let total_liquidity = sol_value_usd + token_value_usd;

    // Validate the result
    validate_liquidity(total_liquidity)?;

    // Round to 2 decimal places
    Ok(round_to_decimals(total_liquidity, 2))
}

/// Convert reserve amount from smallest units to full units.
///
/// # Arguments
///
/// * `amount` - Amount in smallest units
/// * `decimals` - Number of decimal places
///
/// # Returns
///
/// Amount in full units as f64
fn convert_to_full_units(amount: u64, decimals: u8) -> f64 {
    let divisor = 10_f64.powi(decimals as i32);
    amount as f64 / divisor
}

/// Validate that the calculated liquidity is a valid, reasonable value.
///
/// # Arguments
///
/// * `liquidity` - Calculated liquidity value
///
/// # Returns
///
/// * `Ok(())` - If liquidity is valid
/// * `Err(AppError)` - If liquidity is NaN, Infinity, or negative
fn validate_liquidity(liquidity: f64) -> Result<(), AppError> {
    if liquidity.is_nan() {
        return Err(AppError::PriceError(
            "Liquidity calculation resulted in NaN".to_string(),
        ));
    }

    if liquidity.is_infinite() {
        return Err(AppError::PriceError(
            "Liquidity calculation resulted in Infinity".to_string(),
        ));
    }

    if liquidity < 0.0 {
        return Err(AppError::PriceError(format!(
            "Liquidity calculation resulted in negative value: {}",
            liquidity
        )));
    }

    // Log warning if liquidity is suspiciously high
    if liquidity > MAX_REASONABLE_LIQUIDITY_USD {
        log::warn!(
            "Suspiciously high liquidity detected: ${:.2} (exceeds ${:.0} threshold)",
            liquidity,
            MAX_REASONABLE_LIQUIDITY_USD
        );
    }

    Ok(())
}

/// Round a floating point number to a specified number of decimal places.
///
/// # Arguments
///
/// * `value` - The value to round
/// * `decimals` - Number of decimal places
///
/// # Returns
///
/// Rounded value
fn round_to_decimals(value: f64, decimals: u32) -> f64 {
    let multiplier = 10_f64.powi(decimals as i32);
    (value * multiplier).round() / multiplier
}

/// Check if liquidity has drastically changed from the previous snapshot.
///
/// Logs a warning if the change is greater than a threshold percentage.
///
/// # Arguments
///
/// * `previous_liquidity` - Previous liquidity value in USD
/// * `current_liquidity` - Current liquidity value in USD
/// * `threshold_percent` - Threshold percentage for drastic change (e.g., 50.0 for 50%)
pub fn check_liquidity_change(
    previous_liquidity: f64,
    current_liquidity: f64,
    threshold_percent: f64,
) {
    // Skip check if previous liquidity was zero or very small
    if previous_liquidity < 1.0 {
        return;
    }

    let change_percent = ((current_liquidity - previous_liquidity) / previous_liquidity).abs() * 100.0;

    if change_percent > threshold_percent {
        log::warn!(
            "Drastic liquidity change detected: {:.2}% (from ${:.2} to ${:.2})",
            change_percent,
            previous_liquidity,
            current_liquidity
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_to_full_units() {
        // 1 SOL = 1,000,000,000 lamports
        assert_eq!(convert_to_full_units(1_000_000_000, 9), 1.0);
        
        // 10.5 SOL
        assert_eq!(convert_to_full_units(10_500_000_000, 9), 10.5);
        
        // 1000 USDC (6 decimals) = 1,000,000,000 smallest units
        assert_eq!(convert_to_full_units(1_000_000_000, 6), 1000.0);
        
        // 0.5 USDC
        assert_eq!(convert_to_full_units(500_000, 6), 0.5);
    }

    #[test]
    fn test_round_to_decimals() {
        assert_eq!(round_to_decimals(123.456789, 2), 123.46);
        assert_eq!(round_to_decimals(123.454, 2), 123.45);
        assert_eq!(round_to_decimals(100.0, 2), 100.0);
        assert_eq!(round_to_decimals(99.999, 2), 100.0);
    }

    #[test]
    fn test_calculate_liquidity_usd_basic() {
        // Pool with 10 SOL and 1000 USDC
        let result = calculate_liquidity_usd(
            10_000_000_000,  // 10 SOL
            1_000_000_000,   // 1000 USDC (6 decimals)
            100.0,           // SOL = $100
            1.0,             // USDC = $1
            6,
        );
        
        assert!(result.is_ok());
        let liquidity = result.unwrap();
        // (10 * 100) + (1000 * 1) = 2000
        assert_eq!(liquidity, 2000.0);
    }

    #[test]
    fn test_calculate_liquidity_usd_with_9_decimals() {
        // Pool with 5 SOL and 500 tokens (9 decimals like SOL)
        let result = calculate_liquidity_usd(
            5_000_000_000,   // 5 SOL
            500_000_000_000, // 500 tokens
            150.0,           // SOL = $150
            2.0,             // Token = $2
            9,
        );
        
        assert!(result.is_ok());
        let liquidity = result.unwrap();
        // (5 * 150) + (500 * 2) = 750 + 1000 = 1750
        assert_eq!(liquidity, 1750.0);
    }

    #[test]
    fn test_calculate_liquidity_usd_zero_reserves() {
        // Empty pool (valid state)
        let result = calculate_liquidity_usd(0, 0, 100.0, 1.0, 6);
        
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0.0);
    }

    #[test]
    fn test_calculate_liquidity_usd_one_side_zero() {
        // Pool with only SOL
        let result = calculate_liquidity_usd(
            10_000_000_000, // 10 SOL
            0,              // No tokens
            100.0,
            1.0,
            6,
        );
        
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 1000.0); // 10 * 100
    }

    #[test]
    fn test_calculate_liquidity_usd_rounding() {
        // Test rounding to 2 decimal places
        let result = calculate_liquidity_usd(
            1_234_567_890, // ~1.234 SOL
            5_678_901,     // ~5.678 USDC (6 decimals)
            100.567,       // SOL price with decimals
            1.234,         // USDC price with decimals
            6,
        );
        
        assert!(result.is_ok());
        let liquidity = result.unwrap();
        // (1.23456789 * 100.567) + (5.678901 * 1.234) ≈ 124.14 + 7.01 = 131.15
        assert!((liquidity - 131.15).abs() < 0.01);
    }

    #[test]
    fn test_validate_liquidity_nan() {
        let result = validate_liquidity(f64::NAN);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("NaN"));
    }

    #[test]
    fn test_validate_liquidity_infinity() {
        let result = validate_liquidity(f64::INFINITY);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Infinity"));
    }

    #[test]
    fn test_validate_liquidity_negative() {
        let result = validate_liquidity(-100.0);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("negative"));
    }

    #[test]
    fn test_validate_liquidity_valid() {
        assert!(validate_liquidity(0.0).is_ok());
        assert!(validate_liquidity(1000.0).is_ok());
        assert!(validate_liquidity(1_000_000.0).is_ok());
    }

    #[test]
    fn test_validate_liquidity_high_warning() {
        // This should pass validation but trigger a warning log
        // We can't easily test the log output in unit tests, but the function should succeed
        let result = validate_liquidity(2_000_000_000.0); // $2B
        assert!(result.is_ok());
    }

    #[test]
    fn test_check_liquidity_change_no_change() {
        // Should not panic or log warning for small changes
        check_liquidity_change(1000.0, 1050.0, 50.0); // 5% change
    }

    #[test]
    fn test_check_liquidity_change_drastic() {
        // Should log warning for drastic change
        check_liquidity_change(1000.0, 2000.0, 50.0); // 100% change
    }

    #[test]
    fn test_check_liquidity_change_from_zero() {
        // Should not log warning when previous was zero
        check_liquidity_change(0.0, 1000.0, 50.0);
    }

    #[test]
    fn test_calculate_liquidity_real_world_sol_usdc() {
        // Realistic SOL/USDC pool
        // 1000 SOL at $150 = $150,000
        // 150,000 USDC at $1 = $150,000
        // Total = $300,000
        let result = calculate_liquidity_usd(
            1_000_000_000_000, // 1000 SOL
            150_000_000_000,   // 150,000 USDC (6 decimals)
            150.0,
            1.0,
            6,
        );
        
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 300_000.0);
    }

    #[test]
    fn test_calculate_liquidity_different_token_decimals() {
        // Test with various decimal configurations
        
        // 8 decimals (like WBTC)
        let result = calculate_liquidity_usd(
            10_000_000_000, // 10 SOL
            100_000_000,    // 1 token with 8 decimals
            100.0,
            50_000.0,       // Expensive token like BTC
            8,
        );
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 51_000.0); // (10 * 100) + (1 * 50000)

        // 0 decimals (theoretical)
        let result = calculate_liquidity_usd(
            10_000_000_000, // 10 SOL
            1000,           // 1000 tokens with 0 decimals
            100.0,
            1.0,
            0,
        );
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 2_000.0); // (10 * 100) + (1000 * 1)
    }
}
