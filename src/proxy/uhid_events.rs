use std::io;

use log::{debug, error, info, trace, warn};

use crate::codec::{CodecError, PhysicalCodec};
use crate::uhid::UhidEvent;

use super::{
    hex_prefix, is_disconnect_io_error, report_id_label, CachedReportSource, Proxy, DISCONNECTED,
};

impl Proxy {
    pub(super) fn handle_uhid_event(&mut self) -> io::Result<()> {
        loop {
            match self.uhid.recv_event() {
                Ok(Some(event)) => match event {
                    UhidEvent::Start => {
                        info!("virtual HID device started");
                    }
                    UhidEvent::Stop => {
                        return Err(io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            "virtual HID device stopped by kernel",
                        ));
                    }
                    UhidEvent::Open => {
                        debug!("virtual HID device opened by client");
                    }
                    UhidEvent::Close => {
                        debug!("virtual HID device closed by client");
                    }
                    UhidEvent::Output { rtype, ref data } => {
                        if rtype == 1 {
                            trace!(
                                "UHID OUTPUT received: size={}, report_id={}",
                                data.len(),
                                report_id_label(data)
                            );
                            let encoded =
                                self.codec.target.decode_output(data).and_then(|command| {
                                    self.codec
                                        .physical
                                        .encode_output(&command, &mut self.physical_output_state)
                                });
                            match encoded {
                                Ok(encoded) => {
                                    if let Err(e) = self.hidraw.write_output(&encoded) {
                                        if is_disconnect_io_error(&e) {
                                            warn!("failed to write output report: {e}");
                                            info!("controller disconnected");
                                            DISCONNECTED
                                                .store(true, std::sync::atomic::Ordering::SeqCst);
                                            break;
                                        }
                                        error!("failed to write output report: {e}");
                                    }
                                }
                                Err(CodecError::InvalidReport) => {
                                    warn!(
                                        "invalid output report dropped: target={:?}, controller={:?}",
                                        self.codec.target, self.codec.physical
                                    );
                                    warn!(
                                        "output report metadata: rtype={rtype}, size={}, report_id={}",
                                        data.len(),
                                        report_id_label(data)
                                    );
                                }
                            }
                        } else {
                            warn!(
                                "UHID OUTPUT ignored: unexpected rtype={rtype}, size={}, report_id={}",
                                data.len(),
                                report_id_label(data)
                            );
                        }
                    }
                    UhidEvent::GetReport { id, rnum, rtype } => {
                        trace!("UHID GET_REPORT received: id={id}, rnum={rnum}, rtype={rtype}");
                        match self.get_cached_report(rnum) {
                            Some(report) => {
                                match report.source {
                                    CachedReportSource::PhysicalCache => {
                                        trace!("GET_REPORT served from cache: rnum={rnum}");
                                    }
                                    CachedReportSource::TargetFallback => {
                                        trace!(
                                            "GET_REPORT served from target response: rnum={rnum}"
                                        );
                                    }
                                }
                                if let Err(e) = self.uhid.send_get_report_reply(id, 0, &report.data)
                                {
                                    warn!("failed to send GET_REPORT reply: {e}");
                                }
                            }
                            None => {
                                warn!("GET_REPORT unavailable; returning error: rnum={rnum}");
                                if let Err(e) = self.uhid.send_get_report_reply(id, 1, &[]) {
                                    warn!("failed to send GET_REPORT reply: {e}");
                                }
                            }
                        }
                    }
                    UhidEvent::Unknown(event_type) => {
                        warn!("unknown UHID event type: type={event_type}");
                    }
                    UhidEvent::SetReport {
                        id,
                        rnum,
                        rtype,
                        ref data,
                    } => {
                        trace!(
                            "UHID SET_REPORT received: id={id}, rnum={rnum}, rtype={rtype}, size={}, report_id={}",
                            data.len(),
                            report_id_label(data)
                        );
                        let mut reply_err = 0;
                        if rtype == 0 {
                            if let Some(full_data) =
                                self.codec
                                    .physical
                                    .encode_set_report(self.codec.target, rnum, data)
                            {
                                if let Err(e) = self.hidraw.send_feature_report(&full_data) {
                                    warn!("failed to forward SET_REPORT: rnum={rnum}, error={e}");
                                    reply_err = 1;
                                    if is_disconnect_io_error(&e) {
                                        DISCONNECTED
                                            .store(true, std::sync::atomic::Ordering::SeqCst);
                                    }
                                }
                            } else if self.codec.physical == PhysicalCodec::Ds5Bt
                                && self.physical_set_report_unsupported_warned.insert(rnum)
                            {
                                debug!(
                                    "Bluetooth SET_REPORT dropped: unsupported, rnum=0x{rnum:02x}, size={}, report_id={}",
                                    data.len(),
                                    report_id_label(data)
                                );
                                debug!(
                                    "Bluetooth SET_REPORT data: prefix_32={}",
                                    hex_prefix(data, 32)
                                );
                            }
                        }
                        if let Err(e) = self.uhid.send_set_report_reply(id, reply_err) {
                            warn!("failed to send SET_REPORT reply: {e}");
                        }
                        if DISCONNECTED.load(std::sync::atomic::Ordering::SeqCst) {
                            break;
                        }
                    }
                },
                Ok(None) => break,
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}
