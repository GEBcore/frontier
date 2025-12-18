#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

use fp_rent::EvmRentCalculator;
use frame_support::pallet_prelude::*;
use sp_core::H160;
use sp_runtime::traits::SaturatedConversion;

#[frame_support::pallet]
pub mod pallet {
	use super::*;
	use frame_support::{dispatch::DispatchResult, traits::EnsureOrigin};
	use frame_system::{ensure_root, pallet_prelude::OriginFor};

	// ==========================================
	// 1. Configuration & Constants
	// ==========================================

	// 2026-01-01 00:00:00 UTC
	pub const DEFAULT_RENT_START_TIME: u64 = 1_767_225_600_000;
	// 2025-12-10 00:00:00 UTC
	pub const MIN_RENT_START_TIME: u64 = 1_765_324_800_000;
	// 1 satoshi
	pub const SATOSHI: u128 = 10_000_000_000;
	// 10 satoshis = 100 Gwei (10 * 10^10)
	pub const DEFAULT_DAILY_RENT: u128 = 10 * SATOSHI;
	// Milliseconds per day
	pub const MILLISECONDS_PER_DAY: u64 = 86_400_000;

	#[pallet::config]
	pub trait Config: frame_system::Config + pallet_timestamp::Config {
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
		/// A majority of the council can execute some transactions.
		type CouncilOrigin: EnsureOrigin<Self::RuntimeOrigin>;
	}

	#[pallet::pallet]
	pub struct Pallet<T>(_);

	// ==========================================
	// 2. Storage Layer
	// ==========================================

	/// System rent activation timestamp (milliseconds)
	#[pallet::storage]
	#[pallet::getter(fn active_timestamp)]
	pub type ActiveTimestamp<T: Config> = StorageValue<_, u64, ValueQuery, DefaultActiveTimestamp>;

	/// Daily rent amount (default 100 Gwei)
	#[pallet::storage]
	#[pallet::getter(fn daily_rent)]
	pub type DailyRent<T: Config> = StorageValue<_, u128, ValueQuery, DefaultDailyRent>;

	/// Account rent status structure
	#[derive(
		Encode,
		Decode,
		Clone,
		PartialEq,
		Eq,
		RuntimeDebug,
		TypeInfo,
		MaxEncodedLen
	)]
	pub struct RentStatus {
		pub last_rent_paid_time: u64, // Last settlement time
		pub accumulated_rent: u128,   // Total accumulated rent (statistical purpose)
	}

	/// Core storage: H160 -> Rent status
	#[pallet::storage]
	#[pallet::getter(fn account_rent_status)]
	pub type AccountRentMap<T: Config> = StorageMap<_, Twox64Concat, H160, RentStatus, OptionQuery>;

	// Default value implementations
	pub struct DefaultActiveTimestamp;
	impl Get<u64> for DefaultActiveTimestamp {
		fn get() -> u64 {
			DEFAULT_RENT_START_TIME
		}
	}

	pub struct DefaultDailyRent;
	impl Get<u128> for DefaultDailyRent {
		fn get() -> u128 {
			DEFAULT_DAILY_RENT
		}
	}

	// ==========================================
	// 3. Events
	// ==========================================
	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		/// Rent charged: [Account, DaysPaid, Amount]
		RentChargedToBurn(H160, u64, u128),
	}

	#[pallet::error]
	pub enum Error<T> {
		/// Invalid timestamp
		InvalidTimestamp,
		/// Invalid daily rent
		InvalidDailyRent,
	}

	// ==========================================
	// 4. Dispatchable Functions
	// ==========================================
	#[pallet::call]
	impl<T: Config> Pallet<T> {
		#[pallet::call_index(0)]
		#[pallet::weight({0})]
		pub fn set_active_timestamp(origin: OriginFor<T>, timestamp: u64) -> DispatchResult {
			<T as pallet::Config>::CouncilOrigin::try_origin(origin)
				.map(|_| ())
				.or_else(ensure_root)?;

			ensure!(
				timestamp >= MIN_RENT_START_TIME,
				Error::<T>::InvalidTimestamp
			);

			<ActiveTimestamp<T>>::put(timestamp);

			Ok(())
		}

		#[pallet::call_index(1)]
		#[pallet::weight({0})]
		pub fn set_daily_rent(origin: OriginFor<T>, rent: u128) -> DispatchResult {
			<T as pallet::Config>::CouncilOrigin::try_origin(origin)
				.map(|_| ())
				.or_else(ensure_root)?;

			ensure!(rent % SATOSHI == 0, Error::<T>::InvalidDailyRent);

			<DailyRent<T>>::put(rent);

			Ok(())
		}
	}

	// ==========================================
	// 5. Implementation
	// ==========================================

	impl<T: Config> EvmRentCalculator for Pallet<T> {
		/// Calculate and update state, return amount to charge
		/// This method should be called by EVM Adapter (OnChargeEVMTransaction)
		/// return rent value by Wei
		fn process_rent(who: H160) -> u128 {
			let now = <pallet_timestamp::Pallet<T>>::now().saturated_into::<u64>();
			let start_time: u64 = Self::active_timestamp();

			// 1. if zero address(used for eth_call), no charge
			if who == H160::default() {
				return 0;
			}

			// 2. If current time is before rent start time, no charge
			if now < start_time {
				return 0;
			}

			// 3. Get user status
			let mut status = Self::account_rent_status(who).unwrap_or(RentStatus {
				last_rent_paid_time: start_time, // Default to system rent start time
				accumulated_rent: 0,
			});

			// Defensive check: Prevent time regression
			if now <= status.last_rent_paid_time {
				return 0;
			}

			// 4. Calculate elapsed time and days (floor)
			let elapsed_ms = now - status.last_rent_paid_time;
			let days_to_pay = elapsed_ms / MILLISECONDS_PER_DAY;

			// 5. Less than 1 day, no charge, no state update
			if days_to_pay == 0 {
				return 0;
			}

			// 6. Calculate amount
			let daily_rent = Self::daily_rent();
			let rent_amount = (days_to_pay as u128).saturating_mul(daily_rent);

			// 7. Update state
			// Key: Only advance paid days, keep remainder
			let time_paid_for = days_to_pay * MILLISECONDS_PER_DAY;
			status.last_rent_paid_time += time_paid_for;
			status.accumulated_rent = status.accumulated_rent.saturating_add(rent_amount);

			// 8. Write to storage
			<AccountRentMap<T>>::insert(who, status);

			// 9. Emit event
			Self::deposit_event(Event::RentChargedToBurn(who, days_to_pay, rent_amount));

			rent_amount
		}

		/// Corresponds to Solidity: estimateRent(address account)(uint256,uint64)
		/// return rent value by Wei
		fn estimate_rent(who: H160) -> (u128, u64) {
			let now = <pallet_timestamp::Pallet<T>>::now().saturated_into::<u64>();
			let start_time: u64 = Self::active_timestamp();

			if now < start_time {
				return (0, 0);
			}

			let status = Self::account_rent_status(who).unwrap_or(RentStatus {
				last_rent_paid_time: start_time,
				accumulated_rent: 0,
			});

			if now <= status.last_rent_paid_time {
				return (0, 0);
			}

			let elapsed_ms = now - status.last_rent_paid_time;
			let days = elapsed_ms / MILLISECONDS_PER_DAY;

			if days == 0 {
				return (0, 0);
			}

			let rent_amount = (days as u128).saturating_mul(Self::daily_rent());

			(rent_amount, days)
		}
	}
}

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;
