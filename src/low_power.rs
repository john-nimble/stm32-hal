//! This module contains code used to place the STM32L4 in low power modes.
//! Reference section 5.3.3: `Low power modes` of the Reference Manual.

use crate::{
    clocks::{self, InputSrc},
    pac::{PWR, RCC},
};
use cortex_m::{asm::wfi, peripheral::SCB};

// clocks::re_select_input` is separate (in `clocks` instead of here) due to varying significantly
// among families.

// See L4 Reference Manual section 5.3.6. The values correspond
// todo PWR_CR1, LPMS field.
#[derive(Clone, Copy)]
#[repr(u8)]
pub enum StopMode {
    Zero = 0b000,
    One = 0b001,
    Two = 0b010,
}

/// Ref man, table 24
/// Note that this assumes you've already reduced clock frequency below 2 Mhz.
#[cfg(any(feature = "l4", feature = "l5"))]
pub fn low_power_run(pwr: &mut PWR) {
    // Decrease the system clock frequency below 2 MHz
    // LPR = 1
    pwr.cr1.modify(|_, w| w.lpr().set_bit())
}

/// Ref man, table 24
/// Return to normal run mode from low-power run. Requires you to increase the clock speed
/// manually after running this.
#[cfg(any(feature = "l4", feature = "l5"))]
pub fn return_from_low_power_run(pwr: &mut PWR) {
    // LPR = 0
    pwr.cr1.modify(|_, w| w.lpr().clear_bit());

    // Wait until REGLPF = 0
    while pwr.sr2.read().reglpf().bit_is_set() {}

    // Increase the system clock frequency
}

/// Place the system in sleep now mode. To enter `low-power sleep now`, enter low power mode
/// (eg `low_power_mode()`) before running this. Ref man, table 25 and 26
pub fn sleep_now(scb: &mut SCB) {
    // WFI (Wait for Interrupt) (eg `cortext_m::asm::wfi()) or WFE (Wait for Event) while:
    // – SLEEPDEEP = 0
    // – No interrupt (for WFI) or event (for WFE) is pending
    scb.clear_sleepdeep();

    // Or, unimplemented:
    // On return from ISR while:
    // // SLEEPDEEP = 0 and SLEEPONEXIT = 1
    // scb.clear_sleepdeep();
    // scb.set_sleeponexit();

    // Sleep-now: if the SLEEPONEXIT bit is cleared, the MCU enters Sleep mode as soon
    // as WFI or WFE instruction is executed.
    scb.clear_sleeponexit();

    wfi();
}

/// F303 Ref man, table 19.
pub fn sleep_on_exit(scb: &mut SCB) {
    // WFI (Wait for Interrupt) (eg `cortext_m::asm::wfi()) or WFE (Wait for Event) while:

    // SLEEPDEEP = 0 and SLEEPONEXIT = 1
    scb.clear_sleepdeep();
    // Sleep-on-exit: if the SLEEPONEXIT bit is set, the MCU enters Sleep mode as soon
    // as it exits the lowest priority ISR.
    scb.set_sleeponexit();

    wfi();
}

cfg_if::cfg_if! {
    if #[cfg(feature = "f3")] {
        /// Enter `Stop` mode: the middle of the 3 low-power states avail on the
        /// STM32f3.
        /// To exit:  Any EXTI Line configured in Interrupt mode (the corresponding EXTI
        /// Interrupt vector must be enabled in the NVIC). Refer to Table 82.
        /// Ref man, table 20.
        #[cfg(feature = "f3")]
        pub fn stop(scb: &mut SCB, pwr: &mut PWR, input_src: InputSrc, rcc: &mut RCC) {
            //WFI (Wait for Interrupt) or WFE (Wait for Event) while:

            // Set SLEEPDEEP bit in ARM® Cortex®-M4 System Control register
            scb.set_sleepdeep();

            // Clear PDDS bit in Power Control register (PWR_CR)
            // This bit is set and cleared by software. It works together with the LPDS bit.
            // 0: Enter Stop mode when the CPU enters Deepsleep. The regulator status
            // depends on the LPDS bit.
            // 1: Enter Standby mode when the CPU enters Deepsleep.
            pwr.cr.modify(|_, w| w.pdds().clear_bit());

            // Select the voltage regulator mode by configuring LPDS bit in PWR_CR
            // This bit is set and cleared by software. It works together with the PDDS bit.
            // 0: Voltage regulator on during Stop mode
            // 1: Voltage regulator in low-power mode during Stop mode
            // pwr.cr.modify(|_, w| w.pdds().clear_bit());
            pwr.cr.modify(|_, w| w.lpds().set_bit());

            wfi();

            clocks::re_select_input(input_src, rcc);
        }

        /// Enter `Standby` mode: the lowest-power of the 3 low-power states avail on the
        /// STM32f3.
        /// To exit: WKUP pin rising edge, RTC alarm event’s rising edge, external Reset in
        /// NRST pin, IWDG Reset.
        /// Ref man, table 21.
        pub fn standby(scb: &mut SCB, pwr: &mut PWR, input_src: InputSrc, rcc: &mut RCC) {
            // WFI (Wait for Interrupt) or WFE (Wait for Event) while:

            // Set SLEEPDEEP bit in ARM® Cortex®-M4 System Control register
            scb.set_sleepdeep();

            // Set PDDS bit in Power Control register (PWR_CR)
            // This bit is set and cleared by software. It works together with the LPDS bit.
            // 0: Enter Stop mode when the CPU enters Deepsleep. The regulator status
            // depends on the LPDS bit.
            // 1: Enter Standby mode when the CPU enters Deepsleep.
            pwr.cr.modify(|_, w| w.pdds().set_bit());

            // Clear WUF bit in Power Control/Status register (PWR_CSR) (Must do this by setting CWUF bit in
            // PWR_CR.)
            pwr.cr.modify(|_, w| w.cwuf().set_bit());

            wfi();

            clocks::re_select_input(input_src, rcc);
        }

    } else if #[cfg(any(feature = "l4", feature = "l5"))] {
        /// Enter Stop 0, Stop 1, or Stop 2 modes. Reference manual, section 5.3.6. Tables 27, 28, and 29.
        #[cfg(any(feature = "l4", feature = "l5"))]
        pub fn stop(scb: &mut SCB, pwr: &mut PWR, mode: StopMode, input_src: InputSrc, rcc: &mut RCC) {
            // WFI (Wait for Interrupt) or WFE (Wait for Event) while:
            // – SLEEPDEEP bit is set in Cortex®-M4 System Control register
            scb.set_sleepdeep();
            // – No interrupt (for WFI) or event (for WFE) is pending
            // – LPMS = (according to mode) in PWR_CR1
            pwr.cr1.modify(|_, w| unsafe { w.lpms().bits(mode as u8) });

            // Or, unimplemented:
            // On Return from ISR while:
            // – SLEEPDEEP bit is set in Cortex®-M4 System Control register
            // – SLEEPONEXIT = 1
            // – No interrupt is pending
            // – LPMS = “000” in PWR_CR1

            wfi();

            clocks::re_select_input(input_src, rcc);
        }


        /// Enter `Standby` mode. See
        /// Table 30.
        pub fn standby(scb: &mut SCB, pwr: &mut PWR, input_src: InputSrc, rcc: &mut RCC) {
            // – SLEEPDEEP bit is set in Cortex®-M4 System Control register
            scb.set_sleepdeep();
            // – No interrupt (for WFI) or event (for WFE) is pending
            // – LPMS = “011” in PWR_CR1
            pwr.cr1.modify(|_, w| unsafe { w.lpms().bits(0b011) });
            // – WUFx bits are cleared in power status register 1 (PWR_SR1)
            // (Clear by setting cwfuf bits in `pwr_scr`.)
            pwr.scr.write(|w| unsafe { w.bits(0) });
            // todo: Unsure why setting the individual bits isn't working; PWR.scr doesn't have modify method?
            // pwr.scr.modify(|_, w| {
            //     w.cwuf1().set_bit();
            //     w.cwuf2().set_bit();
            //     w.cwuf3().set_bit();
            //     w.cwuf4().set_bit();
            //     w.cwuf5().set_bit();
            // })

            // Or, unimplemented:
            // On return from ISR while:
            // – SLEEPDEEP bit is set in Cortex®-M4 System Control register
            // – SLEEPONEXIT = 1
            // – No interrupt is pending
            // – LPMS = “011” in PWR_CR1 and
            // – WUFx bits are cleared in power status register 1 (PWR_SR1)
            // – The RTC flag corresponding to the chosen wakeup source (RTC Alarm
            // A, RTC Alarm B, RTC wakeup, tamper or timestamp flags) is cleared
            wfi();

            clocks::re_select_input(input_src, rcc);
        }

        /// Enter `Shutdown mode` mode: the lowest-power of the 3 low-power states avail. See
        /// Table 31.
        pub fn shutdown(scb: &mut SCB, pwr: &mut PWR, input_src: InputSrc, rcc: &mut RCC) {
            // – SLEEPDEEP bit is set in Cortex®-M4 System Control register
            scb.set_sleepdeep();
            // – No interrupt (for WFI) or event (for WFE) is pending
            // – LPMS = “011” in PWR_CR1
            pwr.cr1.modify(|_, w| unsafe { w.lpms().bits(0b100) });
            // – WUFx bits are cleared in power status register 1 (PWR_SR1)
            // (Clear by setting cwfuf bits in `pwr_scr`.)
            pwr.scr.write(|w| unsafe { w.bits(0) });
            // todo: Unsure why setting the individual bits isn't working; PWR.scr doesn't have modify method?
            // pwr.scr.modify(|_, w| {
            //     w.cwuf1().set_bit();
            //     w.cwuf2().set_bit();
            //     w.cwuf3().set_bit();
            //     w.cwuf4().set_bit();
            //     w.cwuf5().set_bit();
            // })

            // Or, unimplemented:
            // On return from ISR while:
            // – SLEEPDEEP bit is set in Cortex®-M4 System Control register
            // – SLEEPONEXT = 1
            // – No interrupt is pending
            // – LPMS = “1XX” in PWR_CR1 and
            // – WUFx bits are cleared in power status register 1 (PWR_SR1)
            // – The RTC flag corresponding to the chosen wakeup source (RTC
            // Alarm A, RTC Alarm B, RTC wakeup, tamper or timestamp flags) is
            // cleared
            wfi();

            clocks::re_select_input(input_src, rcc);
        }
    }
}