// Note: This module contains lots of C+P from stm32h7xx-hal.

use core::{cell::UnsafeCell, ops::Deref, ptr};

use super::*;
use crate::{
    check_errors,
    pac::{self, RCC},
    util::RccPeriph,
    MAX_ITERS,
};

/// Possible interrupt types. Enable these in SPIx_IER. Check with SR. Clear with IFCR
#[derive(Copy, Clone)]
pub enum SpiInterrupt {
    /// Additional number of transactions reload interrupt enable (TSERFIE)
    NumberOfTransactionsReload,
    /// Mode fault (MODFIE)
    ModeFault,
    /// TIFRE (TIFREIE)
    Tifre,
    /// CRC error (CRCEIE)
    CrcError,
    /// Overrun (OVRIE)
    Overrun,
    /// Underrun (UNDRIE)
    Underrun,
    /// TXTFIE
    Txtfie,
    /// EOT, SUSP, and TXC (EOTIE)
    EotSuspTxc,
    /// DXP (TXPIE)
    Dxp,
    /// TXP (TXPIE)
    Txp,
    /// RXP (RXPIE)
    Rxp,
}

/// Number of bits in at single SPI data frame. Sets `CFGR1` register, `DSIZE` field.
#[derive(Copy, Clone)]
#[repr(u8)]
pub enum DataSize {
    D4 = 3,
    D5 = 4,
    D6 = 5,
    D7 = 6,
    D8 = 7,
    D9 = 8,
    D10 = 9,
    D11 = 10,
    D12 = 11,
    D13 = 12,
    D14 = 13,
    D15 = 14,
    D16 = 15,
    D17 = 16,
    D18 = 17,
    D19 = 18,
    D20 = 19,
    D21 = 20,
    D22 = 21,
    D23 = 22,
    D24 = 23,
    D25 = 24,
    D26 = 25,
    D27 = 26,
    D28 = 27,
    D29 = 28,
    D30 = 29,
    D31 = 30,
    D32 = 31,
}

impl<R> Spi<R>
where
    R: Deref<Target = pac::spi1::RegisterBlock> + RccPeriph,
{
    /// Initialize an SPI peripheral, including configuration register writes, and enabling and resetting
    /// its RCC peripheral clock.
    pub fn new(regs: R, cfg: SpiConfig, baud_rate: BaudRate) -> Self {
        let rcc = unsafe { &(*RCC::ptr()) };
        R::en_reset(rcc);

        // H743 RM, section 50.4.8: Configuration of SPI.
        // 1. Write the proper GPIO registers: Configure GPIO for MOSI, MISO and SCK pins.
        // (Handled in application code)

        // 2. Write to the SPI_CFG1 and SPI_CFG2 registers to set up proper values of all not
        // reserved bits and bit fields included there with next exceptions:
        // a) SSOM, SSOE, MBR[2:0], MIDI[3:0] and MSSI[3:0] are required at master mode
        // only, the MSSI bits take effect when SSOE is set, MBR setting is required for slave
        // at TI mode, too
        // b) UDRDET[1:0] and UDRCFG[1:0] are required at slave mode only,
        // c) CRCSIZE[4:0] is required if CRCEN is set,
        // d) CPOL, CPHA, LSBFRST, SSOM, SSOE, SSIOP and SSM are not required at TI
        // mode.
        // e) Once the AFCNTR bit is set at SPI_CFG2 register, all the SPI outputs start to be
        // propagated onto the associated GPIO pins regardless the peripheral enable so
        // any later configurations changes of the SPI_CFG1 and SPI_CFG2 registers can
        // affect level of signals at these pins.
        // f) The I2SMOD bit at SPI_I2SCFGR register has to be kept cleared to prevent any
        // unexpected influence of occasional I2S configuration.

        // [St forum thread on how to set up SPI in master mode avoiding mode faults:
        // https://community.st.com/s/question/0D50X0000AFrHS6SQN/stm32h7-what-is-the-proper-
        // way-to-make-spi-work-in-master-mode
        regs.cr1
            .modify(|_, w| w.ssi().bit(cfg.slave_select == SlaveSelect::Software));

        regs.cfg1.modify(|_, w| {
            w.mbr().bits(baud_rate as u8);
            w.dsize().bits(cfg.data_size as u8);
            w.crcen().clear_bit()
        });

        // Specifies minimum time delay (expressed in SPI clock cycles periods) inserted between two
        // consecutive data frames in master mode. In clock cycles; 0 - 15. (hardware CS)
        let inter_word_delay = 0;

        regs.cfg2.modify(|_, w| {
            w.cpol().bit(cfg.mode.polarity as u8 != 0);
            w.cpha().bit(cfg.mode.phase as u8 != 0);
            w.master().set_bit();
            w.ssm().bit(cfg.slave_select == SlaveSelect::Software);
            w.ssoe().bit(cfg.slave_select != SlaveSelect::Software);
            w.midi().bits(inter_word_delay);
            w.master().set_bit();
            w.comm().bits(0b00) // Full-duplex mode
        });

        // 3. Write to the SPI_CR2 register to select length of the transfer, if it is not known TSIZE
        // has to be programmed to zero.
        // Resetting this here; will be set to the appropriate value at each transaction.
        regs.cr2.modify(|_, w| w.tsize().bits(0));

        // 4. Write to SPI_CRCPOLY and into TCRCINI, RCRCINI and CRC33_17 bits at
        // SPI2S_CR1 register to configure the CRC polynomial and CRC calculation if needed.

        // 5. Configure DMA streams dedicated for the SPI Tx and Rx in DMA registers if the DMA
        // streams are used (see chapter Communication using DMA).

        // 6. Program the IOLOCK bit in the SPI_CFG1 register if the configuration protection is
        // required (for safety).

        regs.cr1.modify(|_, w| w.spe().set_bit());

        Self { regs, cfg }
    }

    /// Change the SPI baud rate.
    pub fn reclock(&mut self, baud_rate: BaudRate) {
        self.regs.cr1.modify(|_, w| w.spe().clear_bit());

        self.regs
            .cfg1
            .modify(|_, w| unsafe { w.mbr().bits(baud_rate as u8) });

        self.regs.cr1.modify(|_, w| w.spe().set_bit());
    }

    /// L44 RM, section 40.4.9: "Procedure for disabling the SPI"
    /// When SPI is disabled, it is mandatory to follow the disable procedures described in this
    /// paragraph. It is important to do this before the system enters a low-power mode when the
    /// peripheral clock is stopped. Ongoing transactions can be corrupted in this case. In some
    /// modes the disable procedure is the only way to stop continuous communication running.
    pub fn disable(&mut self) {
        // The correct disable procedure is (except when receive only mode is used):
        // 1. Wait until TXC=1 and/or EOT=1 (no more data to transmit and last data frame sent).
        // When CRC is used, it is sent automatically after the last data in the block is processed.
        // TXC/EOT is set when CRC frame is completed in this case. When a transmission is
        // suspended the software has to wait till CSTART bit is cleared.
        while self.regs.sr.read().txc().bit_is_clear() {}
        while self.regs.sr.read().eot().bit_is_clear() {}
        // 2. Read all RxFIFO data (until RXWNE=0 and RXPLVL=00)
        while self.regs.sr.read().rxwne().bit_is_set() || self.regs.sr.read().rxplvl().bits() != 0 {
            unsafe { ptr::read_volatile(&self.regs.rxdr as *const _ as *const u8) };
        }
        // 3. Disable the SPI (SPE=0).
        self.regs.cr1.modify(|_, w| w.spe().clear_bit());
    }

    // todo: Temp C+P from h7xx hal while troubleshooting.
    /// Internal implementation for exchanging a word
    ///
    /// * Assumes the transaction has started (CSTART handled externally)
    /// * Assumes at least one word has already been written to the Tx FIFO
    fn exchange_duplex(&mut self, word: u8) -> Result<u8, SpiError> {
        let status = self.regs.sr.read();
        check_errors!(status);

        let mut i = 0;
        while !self.regs.sr.read().dxp().is_available() {
            i += 1;
            if i >= MAX_ITERS * 10 {
                return Err(SpiError::Hardware);
            }
        }

        // NOTE(write_volatile/read_volatile) write/read only 1 word
        unsafe {
            let txdr = &self.regs.txdr as *const _ as *const UnsafeCell<u8>;
            ptr::write_volatile(UnsafeCell::raw_get(txdr), word);
            return Ok(ptr::read_volatile(&self.regs.rxdr as *const _ as *const u8));
        }
    }
    /// Internal implementation for reading a word
    ///
    /// * Assumes the transaction has started (CSTART handled externally)
    /// * Assumes at least one word has already been written to the Tx FIFO
    fn read_duplex(&mut self) -> Result<u8, SpiError> {
        check_errors!(self.regs.sr.read());

        let mut i = 0;
        while !self.regs.sr.read().rxp().is_not_empty() {
            i += 1;
            if i >= MAX_ITERS * 10 {
                return Err(SpiError::Hardware);
            }
        }

        // NOTE(read_volatile) read only 1 word
        return Ok(unsafe { ptr::read_volatile(&self.regs.rxdr as *const _ as *const u8) });
    }

    /// Write multiple bytes on the SPI line, blocking until complete.
    pub fn write(&mut self, write_words: &[u8]) -> Result<(), SpiError> {
        // both buffers are the same length
        if write_words.is_empty() {
            return Ok(());
        }

        // Depth of FIFO to use. All current SPI implementations
        // have a FIFO depth of at least 8 (see RM0433 Rev 7
        // Table 409.) but pick 4 as a conservative value.
        const FIFO_WORDS: usize = 4;

        // Fill the first half of the write FIFO
        let len = write_words.len();
        let mut write = write_words.iter();
        for _ in 0..core::cmp::min(FIFO_WORDS, len) {
            self.send(*write.next().unwrap());
        }

        // Continue filling write FIFO and emptying read FIFO
        for word in write {
            let _ = self.exchange_duplex(*word);
        }

        // Dummy read from the read FIFO
        for _ in 0..core::cmp::min(FIFO_WORDS, len) {
            let _ = self.read_duplex();
        }

        Ok(())
    }

    /// Read multiple bytes to a buffer, blocking until complete.
    pub fn transfer(&mut self, words: &mut [u8]) -> Result<(), SpiError> {
        if words.is_empty() {
            return Ok(());
        }

        // Depth of FIFO to use. All current SPI implementations
        // have a FIFO depth of at least 8 (see RM0433 Rev 7
        // Table 409.) but pick 4 as a conservative value.
        const FIFO_WORDS: usize = 4;

        // Fill the first half of the write FIFO
        let len = words.len();
        for i in 0..core::cmp::min(FIFO_WORDS, len) {
            self.send(words[i]);
        }

        for i in FIFO_WORDS..len + FIFO_WORDS {
            if i < len {
                // Continue filling write FIFO and emptying read FIFO
                let read_value = self.exchange_duplex(words[i])?;

                words[i - FIFO_WORDS] = read_value;
            } else {
                // Finish emptying the read FIFO
                words[i - FIFO_WORDS] = self.read_duplex()?;
            }
        }

        Ok(())
    }

    fn read(&mut self) -> Result<u8, SpiError> {
        check_errors!(self.regs.sr.read());

        // NOTE(read_volatile) read only 1 word
        return Ok(unsafe { ptr::read_volatile(&self.regs.rxdr as *const _ as *const u8) });
    }

    fn send(&mut self, word: u8) -> Result<(), SpiError> {
        check_errors!(self.regs.sr.read());

        // NOTE(write_volatile) see note above
        unsafe {
            let txdr = &self.regs.txdr as *const _ as *const UnsafeCell<u8>;
            ptr::write_volatile(UnsafeCell::raw_get(txdr), word)
        }
        // write CSTART to start a transaction in
        // master mode
        self.regs.cr1.modify(|_, w| w.cstart().started());

        return Ok(());
    }

    // todo: H7xx c+p above. Baseline code below.

    /// Read a single byte if available, or block until it's available.
    pub fn read2(&mut self) -> Result<u8, SpiError> {
        check_errors!(self.regs.sr.read());

        let mut i = 0;
        while !self.regs.sr.read().rxwne().bit_is_set() {
            i += 1;
            if i >= MAX_ITERS {
                return Err(SpiError::Hardware);
            }
        }

        Ok(unsafe { ptr::read_volatile(&self.regs.rxdr as *const _ as *const u8) })
    }

    /// Write a single byte if available, or block until it's available.
    pub fn write_one(&mut self, byte: u8) -> Result<(), SpiError> {
        check_errors!(self.regs.sr.read());

        let mut i = 0;
        while !self.regs.sr.read().txc().bit_is_set() {
            i += 1;
            if i >= MAX_ITERS {
                return Err(SpiError::Hardware);
            }
        }

        #[allow(invalid_reference_casting)]
        unsafe {
            ptr::write_volatile(&self.regs.txdr as *const _ as *mut u8, byte)
        };

        Ok(())
    }

    /// Enable an interrupt.
    pub fn enable_interrupt(&mut self, interrupt_type: SpiInterrupt) {
        self.regs.ier.modify(|_, w| match interrupt_type {
            SpiInterrupt::NumberOfTransactionsReload => w.tserfie().set_bit(),
            SpiInterrupt::ModeFault => w.modfie().set_bit(),
            SpiInterrupt::Tifre => w.tifreie().set_bit(),
            SpiInterrupt::CrcError => w.crceie().set_bit(),
            SpiInterrupt::Overrun => w.ovrie().set_bit(),
            SpiInterrupt::Underrun => w.udrie().set_bit(),
            SpiInterrupt::Txtfie => w.txtfie().set_bit(),
            SpiInterrupt::EotSuspTxc => w.eotie().set_bit(),
            // SpiInterrupt::Dxp => w.dxpie().set_bit(),
            // SpiInterrupt::Txp => w.txpie().set_bit(),
            // SpiInterrupt::Rxp => w.rxpie().set_bit(),
            _ => w.eotie().set_bit(), // todo: PAC ommission?
        });
    }

    /// Clear an interrupt.
    pub fn clear_interrupt(&mut self, interrupt_type: SpiInterrupt) {
        self.regs.ifcr.write(|w| match interrupt_type {
            SpiInterrupt::NumberOfTransactionsReload => w.tserfc().set_bit(),
            SpiInterrupt::ModeFault => w.modfc().set_bit(),
            SpiInterrupt::Tifre => w.tifrec().set_bit(),
            SpiInterrupt::CrcError => w.crcec().set_bit(),
            SpiInterrupt::Overrun => w.ovrc().set_bit(),
            SpiInterrupt::Underrun => w.udrc().set_bit(),
            SpiInterrupt::Txtfie => w.txtfc().set_bit(),
            SpiInterrupt::EotSuspTxc => w.eotc().set_bit(),
            // SpiInterrupt::Dxp => w.dxpc().set_bit(),
            // SpiInterrupt::Txp => w.txpc().set_bit(),
            // SpiInterrupt::Rxp => w.rxpc().set_bit(),
            _ => w.eotc().set_bit(), // todo: PAC ommission?
        });
    }
}
