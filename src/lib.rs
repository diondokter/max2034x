#![no_std]
#![doc = include_str!("../README.md")]

use core::{fmt::Debug, marker::PhantomData};
use device_driver::{
    ll::{register_async::RegisterInterfaceAsync, LowLevelDevice},
    Bit,
};
use devices::DeviceVersion;
use error::DeviceError;
use ll::{HardwareInterface, Max2034xLL};
use state::{Disabled, Enabled, InitializedState, State, Uninitialized};
pub use types::*;

use embedded_hal::digital::{InputPin, OutputPin};
use embedded_hal_async::i2c::I2c;

/// Version-specific declarations
pub mod devices;

pub mod error;
pub mod ll;
pub mod types;

/// Pin struct for the boost fast pin and the interrupt pin.
pub struct Pins<BF, BI> {
    pub boost_fast: Option<BF>,
    pub boost_nint: Option<BI>,
}

/// Max2034x device driver.
pub struct Max2034x<I: HardwareInterface, BF, BI, S: State> {
    ll: Max2034xLL<I>,
    pins: Pins<BF, BI>,
    inductor: Inductor,
    _marker: PhantomData<S>,
}

type Result<T, I> =
    core::result::Result<T, DeviceError<<I as RegisterInterfaceAsync>::InterfaceError>>;
type NewDeviceResult<V, I2C, BF, BI, BS> =
    Result<Max2034x<ll::Max2034xInterface<V, I2C>, BF, BI, BS>, ll::Max2034xInterface<V, I2C>>;

impl<V, I2C, EBUS, BF, BI> Max2034x<ll::Max2034xInterface<V, I2C>, BF, BI, Uninitialized>
where
    V: DeviceVersion,
    I2C: I2c<Error = EBUS>,
    EBUS: Debug,
    BF: OutputPin,
    BI: InputPin,
{
    /// Create a new device instance. Creates a new low-level Max2034xInterface, and
    /// calls [`Self::with_interface`], passing the low-level interface.
    pub async fn new(
        i2c: I2C,
        version: V,
        pins: Pins<BF, BI>,
        inductor: Inductor,
    ) -> NewDeviceResult<V, I2C, BF, BI, V::BootState> {
        let ll = ll::Max2034xInterface::new(i2c, version);
        Self::with_interface(ll, pins, inductor).await
    }
}

impl<I: HardwareInterface, BF: OutputPin, BI: InputPin> Max2034x<I, BF, BI, Uninitialized> {
    /// Create a new device instance with the passed low level interface.
    /// Verifies the device ID internally, and sets the FET scale
    /// according to the passed Inductor if it does not correspond
    /// to the default value of the BBstFETScale register of the device version.  
    ///
    /// For initally enabled devices (BBstEn = Enabled in Table 3 of the datasheet), you
    /// need to disable and re-enable the device in order for the FET scale
    /// update to take effect if it differs from the default.
    pub async fn with_interface(
        interface: I,
        pins: Pins<BF, BI>,
        inductor: Inductor,
    ) -> Result<Max2034x<I, BF, BI, I::BootState>, I> {
        let mut buck_boost = Self {
            ll: Max2034xLL::new(interface),
            pins,
            inductor,
            _marker: PhantomData,
        };

        let chip_id = buck_boost.ll.registers().chip_id().read().await?.id();
        if chip_id != I::CHIP_ID {
            return Err(DeviceError::BadDeviceId);
        }

        // We rely on a correct value of BBstFETScale
        // so we update it on initialization if needed.
        if buck_boost.inductor != I::DEFAULT_INDUCTOR_CONFIG {
            let fet_scale = match buck_boost.inductor {
                Inductor::L1uH => Bit::Cleared,
                Inductor::L2_2uH => Bit::Set,
            };
            buck_boost
                .ll
                .registers()
                .b_bst_cfg1()
                .modify(|_, w| w.b_bst_fet_scale(fet_scale))
                .await?;
        }

        Ok(buck_boost.into_state())
    }
}

impl<I: HardwareInterface, BF: OutputPin, BI: InputPin, S: State> Max2034x<I, BF, BI, S> {
    /// Helper method to alter the state.
    fn into_state<N: State>(self) -> Max2034x<I, BF, BI, N> {
        Max2034x {
            ll: self.ll,
            pins: self.pins,
            inductor: self.inductor,
            _marker: PhantomData,
        }
    }

    /// Get the device's Chip ID. See Table 3
    /// in the Datasheet for expected values
    pub async fn get_chip_id(&mut self) -> Result<u8, I> {
        let id = self.ll.registers().chip_id().read().await?.id();
        Ok(id)
    }

    pub fn free(self) -> (ll::Max2034xLL<I>, Pins<BF, BI>) {
        (self.ll, self.pins)
    }
}

impl<I: HardwareInterface, BF: OutputPin, BI: InputPin> Max2034x<I, BF, BI, Disabled> {
    /// Enable device output power. If succesful, returns an enabled device.
    pub async fn enable(mut self) -> Result<Max2034x<I, BF, BI, Enabled>, I> {
        self.ll
            .registers()
            .b_bst_cfg0()
            .modify(|_, w| w.b_bst_en(Bit::Set))
            .await?;
        Ok(self.into_state())
    }

    /// Enable or disable zero crossing comparator.
    pub async fn enable_zero_crossing_comparator(&mut self, enabled: bool) -> Result<(), I> {
        self.ll
            .registers()
            .b_bst_cfg0()
            .modify(|_, w| w.b_bst_zc_cmp_dis(Bit::from(!enabled)))
            .await?;
        Ok(())
    }

    /// Enable or disable pass through mode.
    pub async fn enable_pass_through_mode(&mut self, enabled: bool) -> Result<(), I> {
        self.ll
            .registers()
            .b_bst_cfg1()
            .modify(|_, w| w.pas_thr_mode(Bit::from(enabled)))
            .await?;
        Ok(())
    }

    /// Enable or disable integrator.
    pub async fn enable_integrator(&mut self, enabled: bool) -> Result<(), I> {
        self.ll
            .registers()
            .b_bst_cfg1()
            .modify(|_, w| w.b_bst_integ_en(Bit::from(enabled)))
            .await?;
        Ok(())
    }

    /// Set buck-boost mode.
    pub async fn set_buck_boost_mode(&mut self, mode: BuckBoostMode) -> Result<(), I> {
        self.ll
            .registers()
            .b_bst_cfg0()
            .modify(|_, w| w.b_bst_mode(mode))
            .await?;
        Ok(())
    }
}

impl<I: HardwareInterface, BF: OutputPin, BI: InputPin> Max2034x<I, BF, BI, Enabled> {
    /// Disable device output power. If succesful, returns a disabled device.
    pub async fn disable(mut self) -> Result<Max2034x<I, BF, BI, Disabled>, I> {
        self.ll
            .registers()
            .b_bst_cfg0()
            .modify(|_, w| w.b_bst_en(Bit::Cleared))
            .await?;
        Ok(self.into_state())
    }
}

impl<I: HardwareInterface, BF: OutputPin, BI: InputPin, S: InitializedState>
    Max2034x<I, BF, BI, S>
{
    /// Get access to the low level registers. This is useful for when you need to change
    /// a register, but the high level interface hasn't implemented it yet.
    /// Note: using the low-level interface you may break the driver typestate.
    pub fn ll(&mut self) -> &mut Max2034xLL<I> {
        &mut self.ll
    }

    /// Enable or disable gradually ramping up the output voltage.
    pub async fn enable_ramp(&mut self, enabled: bool) -> Result<(), I> {
        self.ll
            .registers()
            .b_bst_cfg0()
            .modify(|_, w| w.b_bst_ramp_en(Bit::from(enabled)))
            .await?;
        Ok(())
    }

    /// Enable or disable low EMI mode.
    pub async fn enable_low_emi(&mut self, enabled: bool) -> Result<(), I> {
        self.ll
            .registers()
            .b_bst_cfg0()
            .modify(|_, w| w.b_bst_low_emi(Bit::from(enabled)))
            .await?;
        Ok(())
    }

    /// Enable or disable active discharge.
    pub async fn enable_active_discharge(&mut self, enabled: bool) -> Result<(), I> {
        self.ll
            .registers()
            .b_bst_cfg0()
            .modify(|_, w| w.b_bst_act_dsc(Bit::from(enabled)))
            .await?;
        Ok(())
    }

    /// Enable or disable passive discharge.
    pub async fn enable_passive_discharge(&mut self, enabled: bool) -> Result<(), I> {
        self.ll
            .registers()
            .b_bst_cfg0()
            .modify(|_, w| w.b_bst_psv_dsc(Bit::from(enabled)))
            .await?;
        Ok(())
    }

    /// Enable or disable using the fast boost pin to toggle fast boost mode.
    pub async fn enable_fast_boost_pin(&mut self, enabled: bool) -> Result<(), I> {
        self.ll
            .registers()
            .b_bst_cfg1()
            .modify(|_, w| w.fst_cmp_en(Bit::from(enabled)))
            .await?;
        Ok(())
    }

    /// Set force switch over mode.
    pub async fn set_force_switch_over(&mut self, mode: SwitchOverMode) -> Result<(), I> {
        self.ll
            .registers()
            .b_bst_cfg1()
            .modify(|_, w| w.swo_frc_in(mode))
            .await?;
        Ok(())
    }

    /// Enable or disable fast boost by register.
    pub async fn enable_fast_boost_by_register(&mut self, enabled: bool) -> Result<(), I> {
        self.ll
            .registers()
            .b_bst_cfg0()
            .modify(|_, w| w.b_bst_fast(enabled.into()))
            .await?;

        Ok(())
    }

    /// Enable or disable fast boost using the boost_fast pin.
    /// Does nothing if the passed pin is `None`.
    /// Be sure to enable the fast boost bin using
    /// [`Self::enable_fast_boost_pin`].
    pub fn enable_fast_boost(&mut self, enabled: bool) -> core::result::Result<(), BF::Error> {
        match &mut self.pins.boost_fast {
            Some(p) => p.set_state(enabled.into()),
            None => Ok(()),
        }
    }

    /// Read `boost_nint` pin level, indicating whether an interrupt is active.
    /// Returns `false` if the passed pin in `None`.
    pub fn interrupt_active(&mut self) -> core::result::Result<bool, BI::Error> {
        match &mut self.pins.boost_nint {
            Some(p) => p.is_low(),
            None => Ok(false),
        }
    }

    /// Read the interrupt cause register.
    pub async fn get_interrupt_cause(&mut self) -> Result<InterruptStatus, I> {
        let int = self.ll.registers().int().read().await?.int().unwrap();
        Ok(int)
    }

    /// Read the status register.
    pub async fn get_status(&mut self) -> Result<InterruptStatus, I> {
        let status = self.ll.registers().status().read().await?.status().unwrap();
        Ok(status)
    }

    /// Enable an interrupt. Use [`InterruptStatus::Both`] to enable both interrupts.
    pub async fn enable_interrupt(&mut self, interrupt: InterruptStatus) -> Result<(), I> {
        self.ll
            .registers()
            .mask()
            .modify(|_, w| w.mask(!interrupt))
            .await?;
        Ok(())
    }

    async fn lock_vset(&mut self, locked: bool) -> Result<(), I> {
        self.ll
            .registers()
            .lock_msk()
            .modify(|_, w| w.b_bst_lck(Bit::Cleared))
            .await?;

        let passwd = if locked { 0xAA } else { 0x55 };

        self.ll
            .registers()
            .lock_unlock()
            .modify(|_, w| w.passwd(passwd))
            .await?;
        self.ll
            .registers()
            .lock_msk()
            .modify(|_, w| w.b_bst_lck(Bit::Set))
            .await?;
        Ok(())
    }

    /// Sets the device output voltage. Uses figure 5 of the datasheet
    /// to also set recommended peak current limits. Also takes care
    /// of locking and unlocking the BBstVSet register and masking and
    /// unmasking from locking using the LockMsk register.
    pub async fn set_output_voltage(&mut self, v_out: OutputVoltage) -> Result<(), I> {
        // Below values are based on figure 5 of the datasheet.
        let (step_up_raw, step_down_raw) = match v_out.millivolts() {
            2500..=2749 => (0b0100, 0b1010),
            2750..=3124 => (0b0100, 0b1001),
            3125..=3499 => (0b0100, 0b1000),
            3500..=3624 => (0b0101, 0b1000),
            3625..=3874 => (0b0101, 0b0111),
            3875..=4249 => (0b0110, 0b0111),
            4250..=4499 => (0b0111, 0b0111),
            4500..=4624 => (0b0111, 0b0110),
            4625..=4999 => (0b1000, 0b0110),
            5000..=5374 => (0b1001, 0b0110),
            5375..=5500 => (0b1010, 0b0110),
            _ => unreachable!("Invalid OutputVoltage value"),
        };
        // We follow the recommended values here (figure 5),
        // so no further need to check for safety.
        self.set_raw_peak_current_limits(step_up_raw, step_down_raw)
            .await?;

        self.lock_vset(false).await?;

        let res = self
            .ll
            .registers()
            .b_bst_v_set()
            .modify(|_, w| w.b_bst_v_set(v_out.raw))
            .await;
        self.lock_vset(true).await?;

        // make sure we lock again before returning an error.
        res?;
        Ok(())
    }

    /// Set inductor peak current limits. `step_up` is used whenever
    /// V<sub>IN</sub> < V<sub>OUT</sub>, `step_down` is used whenever
    /// V<sub>IN</sub> > V<sub>OUT</sub>. Enforces the safe operating area
    /// of these values according to figure 4 in the datasheet, by
    /// clipping the step_down value.
    pub async fn set_peak_current_limits(
        &mut self,
        step_up: CurrentLimit,   // BBstIPSet1
        step_down: CurrentLimit, // BBstIPSet2
    ) -> Result<(), I> {
        let step_down_minimum_raw = match step_up.raw(self.inductor) {
            0b0000..=0b0010 => 0b1010,
            0b0011..=0b0011 => 0b1001,
            0b0100..=0b0101 => 0b1000,
            0b0110..=0b0111 => 0b0111,
            0b1000..=0b1111 => 0b0110,
            _ => unreachable!("Invalid CurrentLimit value"),
        };

        // Value safety is enforced by clipping step_down_raw to the minimum.
        self.set_raw_peak_current_limits(
            step_up.raw(self.inductor),
            step_down.raw(self.inductor).max(step_down_minimum_raw),
        )
        .await
    }

    /// Sets peak current limits from raw values.
    /// Make sure these values are within the bounds specified
    /// figure 4 of the datasheet.
    async fn set_raw_peak_current_limits(
        &mut self,
        step_up_raw: u8,
        step_down_raw: u8,
    ) -> Result<(), I> {
        self.ll
            .registers()
            .b_bst_i_set()
            .modify(|_, w| w.b_bst_ip_set1(step_up_raw).b_bst_ip_set2(step_down_raw))
            .await?;

        Ok(())
    }

    /// Set switching frequency threshold.
    pub async fn set_switch_freq_threshold(&mut self, f_ths: FrequencyThreshold) -> Result<(), I> {
        self.ll
            .registers()
            .b_bst_v_set()
            .modify(|_, w| w.b_bst_fhigh_sh(f_ths))
            .await?;
        Ok(())
    }

    /// Set the `BBstIPAdptDis` adaptive peak/valley current adjustment
    pub async fn set_adaptive_current_adjustment(&mut self, enabled: bool) -> Result<(), I> {
        self.ll
            .registers()
            .b_bst_cfg1()
            .modify(|_, w| w.b_bst_ip_adpt_dis(Bit::from(!enabled)))
            .await?;
        Ok(())
    }
}

pub mod state {
    //! Device state definitions, used for typestate setup.

    /// A state that indicates the device was initialized.
    pub trait InitializedState: State {}
    /// General device state. Cannot be implemented by users.
    pub trait State: Sealed {}

    macro_rules! state {
        ($state:ident, $doc:literal, true) => {
            state!($state, $doc);
            impl InitializedState for $state {}
        };
        ($state:ident, $doc:literal) => {
            #[doc = $doc]
            #[derive(Debug)]
            pub struct $state;
            impl Sealed for $state {}
            impl State for $state {}
        };
    }

    use self::sealed::Sealed;
    mod sealed {
        pub trait Sealed {}
    }

    state!(Uninitialized, "Uninitialized device");
    state!(Disabled, "Buck-boost enabled", true);
    state!(Enabled, "Buck-boost disabled", true);
}
