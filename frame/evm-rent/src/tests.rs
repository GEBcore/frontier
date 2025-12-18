use super::mock::*;
use crate::pallet::*;
use fp_rent::EvmRentCalculator;
use frame_support::{assert_noop, assert_ok};
use sp_core::H160;

#[test]
fn set_active_timestamp_works() {
	new_test_ext().execute_with(|| {
		let new_time = MIN_RENT_START_TIME + 1000;
		assert_noop!(
			Rent::set_active_timestamp(RuntimeOrigin::signed(1), new_time),
			sp_runtime::DispatchError::BadOrigin
		);
		assert_ok!(Rent::set_active_timestamp(RuntimeOrigin::root(), new_time));
		assert_eq!(Rent::active_timestamp(), new_time);
		assert_noop!(
			Rent::set_active_timestamp(RuntimeOrigin::root(), MIN_RENT_START_TIME - 1),
			Error::<Test>::InvalidTimestamp
		);
	});
}

#[test]
fn set_daily_rent_works() {
	new_test_ext().execute_with(|| {
		let valid_rent = 2 * SATOSHI;
		let invalid_rent = SATOSHI - 1;

		assert_ok!(Rent::set_daily_rent(RuntimeOrigin::root(), valid_rent));
		assert_eq!(Rent::daily_rent(), valid_rent);

		assert_noop!(
			Rent::set_daily_rent(RuntimeOrigin::root(), invalid_rent),
			Error::<Test>::InvalidDailyRent
		);
	});
}

#[test]
fn process_rent_logic_flow() {
	new_test_ext().execute_with(|| {
		let alice = H160::from_low_u64_be(1);
		let start_time = DEFAULT_RENT_START_TIME; // 1_767_225_600_000

		// 1. Before start time, rent should be 0
		set_time(start_time - 1000);
		assert_eq!(Rent::process_rent(alice), 0);

		// 2. Exactly at start time, but less than 1 day
		set_time(start_time);
		assert_eq!(Rent::process_rent(alice), 0);

		// 3. After 1.5 days, charge 1 day rent, update last settlement to start_time + 1 day
		set_time(start_time + (MILLISECONDS_PER_DAY * 3 / 2));
		let expected_rent = DEFAULT_DAILY_RENT;
		assert_eq!(Rent::process_rent(alice), expected_rent);

		// Verify state update
		let status = Rent::account_rent_status(alice).unwrap();
		assert_eq!(
			status.last_rent_paid_time,
			start_time + MILLISECONDS_PER_DAY
		);
		assert_eq!(status.accumulated_rent, expected_rent);

		// 4. Another 0.8 days (total 2.3 days), 1 day since last settlement, charge and update to start_time + 2 days
		set_time(start_time + (MILLISECONDS_PER_DAY * 23 / 10));
		assert_eq!(Rent::process_rent(alice), DEFAULT_DAILY_RENT);

		let status = Rent::account_rent_status(alice).unwrap();
		assert_eq!(
			status.last_rent_paid_time,
			start_time + 2 * MILLISECONDS_PER_DAY
		);
		assert_eq!(status.accumulated_rent, expected_rent * 2);

		// 5. Another 1.2 days (total 3.5 days), 1.5 days since last settlement
		// Charge 1 day, total accumulated should be 3 days
		set_time(start_time + (MILLISECONDS_PER_DAY * 35 / 10));
		assert_eq!(Rent::process_rent(alice), DEFAULT_DAILY_RENT);

		let status = Rent::account_rent_status(alice).unwrap();
		assert_eq!(
			status.last_rent_paid_time,
			start_time + 3 * MILLISECONDS_PER_DAY
		);
		assert_eq!(status.accumulated_rent, DEFAULT_DAILY_RENT * 3);
	});
}

#[test]
fn estimate_rent_matches_process() {
	new_test_ext().execute_with(|| {
		let bob = H160::from_low_u64_be(2);
		let start_time = DEFAULT_RENT_START_TIME;

		set_time(start_time + (MILLISECONDS_PER_DAY * 5));

		let (est_amount, est_days) = Rent::estimate_rent(bob);
		assert_eq!(est_days, 5);
		assert_eq!(est_amount, DEFAULT_DAILY_RENT * 5);

		// Execute actual rent charging
		let actual_amount = Rent::process_rent(bob);
		assert_eq!(actual_amount, est_amount);
	});
}

#[test]
fn zero_address_is_exempt() {
	new_test_ext().execute_with(|| {
		let zero_addr = H160::default();
		set_time(DEFAULT_RENT_START_TIME + (MILLISECONDS_PER_DAY * 10));
		assert_eq!(Rent::process_rent(zero_addr), 0);
	});
}
