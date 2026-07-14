use std::sync::{Arc, RwLock};

use dseuhid::{config, control, mapping, shutdown::ShutdownSignal};
use log::{debug, error, info, warn};

use crate::codec::{CodecPipeline, FeatureReportCache};
use crate::daemon::reject_inactive_control;
use crate::device::{self, DeviceInfo, HidrawDevice};
use crate::keyboard::KeyboardDevice;
use crate::proxy::{self, Proxy, ProxyInit};
use crate::uhid::UhidDevice;

pub(crate) enum SessionExit {
    ConfigChanged(Option<config::ActiveConfig>),
    DeviceGone { reset_config: bool },
    Shutdown,
    Fatal,
}

fn is_controller_gone_error(error: &std::io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::ENOENT | libc::EIO | libc::ENODEV | libc::ENXIO)
    )
}

pub(crate) fn run(
    dev_info: &DeviceInfo,
    active_config: &Option<config::ActiveConfig>,
    shutdown: &ShutdownSignal,
    control: &mut control::ControlServer,
) -> SessionExit {
    let loaded_config = match active_config.as_ref() {
        Some(active) => {
            info!("loading config: source={}", active.source());
            match active.parse() {
                Ok(cfg) => Some(cfg),
                Err(e) => {
                    error!("failed to parse active config: {e}");
                    return SessionExit::Fatal;
                }
            }
        }
        None => None,
    };
    let output_device = loaded_config
        .as_ref()
        .map_or_else(|| "auto".to_string(), |cfg| cfg.output_device.clone());
    let codec_pipeline =
        CodecPipeline::from_device_and_output(dev_info.kind, dev_info.transport, &output_device);
    let mapping = match loaded_config.as_ref() {
        Some(cfg) => match cfg.to_mapping_config() {
            Ok(mapping) => {
                for name in config::ALL_BUTTON_NAMES {
                    if !cfg.buttons.contains_key(*name) {
                        debug!("button not configured; using passthrough: button={name}");
                    }
                }
                proxy::warn_ignored_edge_passthroughs(cfg, dev_info.kind, codec_pipeline.target);
                mapping
            }
            Err(e) => {
                error!("failed to build mapping: {e}");
                return SessionExit::Fatal;
            }
        },
        None => {
            info!("no config specified; using passthrough mode");
            mapping::MappingConfig::default()
        }
    };
    let mapping = Arc::new(RwLock::new(mapping));

    let mut hidraw = match HidrawDevice::open(&dev_info.path) {
        Ok(device) => device,
        Err(e) if is_controller_gone_error(&e) => {
            warn!("failed to open controller: {e}");
            return SessionExit::DeviceGone {
                reset_config: false,
            };
        }
        Err(e) => {
            error!("failed to open hidraw device: {e}");
            return SessionExit::Fatal;
        }
    };

    let mut uhid = match UhidDevice::open() {
        Ok(device) => device,
        Err(e) => {
            error!("failed to open /dev/uhid: {e}");
            error!("UHID kernel module may be unavailable; verify with: modprobe uhid");
            return SessionExit::Fatal;
        }
    };

    let mut report_cache = FeatureReportCache::new();
    for request in codec_pipeline
        .physical
        .feature_reports_to_cache(codec_pipeline.target)
    {
        let mut buf = vec![request.report_id];
        buf.resize(request.size, 0);
        if device::ioctl_get_feature_report(hidraw.as_raw_fd(), &mut buf).is_ok() {
            match codec_pipeline.physical.decode_feature_report(*request, buf) {
                Ok(data) => {
                    debug!(
                        "feature report cached: report_id=0x{:02x}",
                        request.report_id
                    );
                    report_cache.insert(request.report_id, data);
                }
                Err(_) => {
                    warn!(
                        "invalid feature report; using target response: report_id=0x{:02x}",
                        request.report_id
                    );
                }
            }
        } else {
            warn!(
                "failed to read feature report; using target response: report_id=0x{:02x}",
                request.report_id
            );
        }
    }
    codec_pipeline
        .target
        .seed_feature_reports(&mut report_cache);

    let target_identity = codec_pipeline
        .target
        .usb_identity(dev_info, hidraw.report_descriptor());
    if let Err(e) = uhid.create(
        &target_identity.name,
        "",
        target_identity.uniq,
        0x0003,
        dev_info.vid as u32,
        target_identity.product_id,
        0x0100,
        0,
        target_identity.report_descriptor,
    ) {
        error!("failed to create virtual HID device: {e}");
        return SessionExit::Fatal;
    }

    info!(
        "virtual HID device created: name={}, output={}",
        target_identity.name, target_identity.label
    );

    if let Err(e) = hidraw.restrict_evdev_nodes() {
        warn!("failed to restrict input nodes: {e}");
    }

    info!("proxy initializing");
    let keyboard = match KeyboardDevice::open() {
        Ok(keyboard) => {
            info!("virtual keyboard created");
            keyboard
        }
        Err(e) => {
            warn!("virtual keyboard unavailable; keyboard targets disabled: {e}");
            KeyboardDevice::dummy()
        }
    };

    if let Err(e) = reject_inactive_control(control) {
        error!("failed to reject control request while proxy is inactive: {e}");
        return SessionExit::Fatal;
    }
    let mut proxy = Proxy::new(ProxyInit {
        hidraw,
        uhid,
        keyboard,
        mapping,
        active_config: active_config.clone(),
        report_cache,
        codec: codec_pipeline,
        source_kind: dev_info.kind,
        output_device_config: output_device,
    });
    match proxy.run(shutdown, control) {
        proxy::ExitReason::ConfigChanged => {
            SessionExit::ConfigChanged(proxy.active_config().cloned())
        }
        proxy::ExitReason::DeviceGone => {
            proxy.forget_restore_on_physical_disconnect();
            drop(proxy);
            SessionExit::DeviceGone { reset_config: true }
        }
        proxy::ExitReason::UserShutdown => SessionExit::Shutdown,
        proxy::ExitReason::FatalError => SessionExit::Fatal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controller_open_retries_only_device_gone_errors() {
        for errno in [libc::ENOENT, libc::EIO, libc::ENODEV, libc::ENXIO] {
            assert!(is_controller_gone_error(
                &std::io::Error::from_raw_os_error(errno)
            ));
        }
        for errno in [libc::EACCES, libc::EINVAL, libc::EBADF] {
            assert!(!is_controller_gone_error(
                &std::io::Error::from_raw_os_error(errno)
            ));
        }
    }
}
