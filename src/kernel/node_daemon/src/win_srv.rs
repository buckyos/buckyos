use crate::node_daemon;
use clap::{Arg, ArgMatches, Command};
use lazy_static::lazy_static;
use std::cell::OnceCell;
use std::ffi::OsString;
use std::process::exit;
use std::sync::OnceLock;
use std::time::Duration;
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{ServiceControlHandlerResult, ServiceStatusHandle};
use windows_service::{define_windows_service, service_control_handler, service_dispatcher};

define_windows_service!(ffi_service_main, service_main);

struct WinService {
    status_handle: Option<ServiceStatusHandle>,
    cur_svc_status: ServiceStatus,
}

lazy_static! {
    static ref SERVICE: std::sync::RwLock<WinService> = std::sync::RwLock::new(WinService {
        status_handle: None,
        cur_svc_status: ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: ServiceState::StartPending,
            controls_accepted: ServiceControlAccept::empty(),
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        }
    });
}

static MATCHES: OnceLock<ArgMatches> = OnceLock::new();

pub(crate) fn service_main(_arguments: Vec<OsString>) -> windows_service::Result<()> {
    let status_handle = service_control_handler::register("buckyos", move |event| {
        match event {
            ServiceControl::Stop => {
                let mut service = SERVICE.write().unwrap();

                service.cur_svc_status.current_state = ServiceState::Stopped;
                service.cur_svc_status.controls_accepted = ServiceControlAccept::empty();

                service
                    .status_handle
                    .unwrap()
                    .set_service_status(service.cur_svc_status.clone());

                //exit(0);
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => {
                let mut status = SERVICE.read().unwrap().cur_svc_status.clone();
                status.process_id = Some(std::process::id());
                SERVICE
                    .read()
                    .unwrap()
                    .status_handle
                    .unwrap()
                    .set_service_status(status);
                ServiceControlHandlerResult::NoError
            }
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    })?;

    {
        let mut service = SERVICE.write().unwrap();

        service.cur_svc_status.current_state = ServiceState::Running;
        service.cur_svc_status.controls_accepted = ServiceControlAccept::STOP;
        service.status_handle = Some(status_handle);

        service
            .status_handle
            .unwrap()
            .set_service_status(service.cur_svc_status.clone());
    }

    let matches = MATCHES.get().unwrap().clone();

    let run_result = node_daemon::run(matches);
    if let Err(err) = run_result {
        let mut service = SERVICE.write().unwrap();
        service.cur_svc_status.current_state = ServiceState::Stopped;
        service.cur_svc_status.controls_accepted = ServiceControlAccept::empty();
        service.cur_svc_status.exit_code = ServiceExitCode::Win32(1);
        if let Err(set_status_err) = service
            .status_handle
            .as_ref()
            .expect("windows service status handle not initialized")
            .set_service_status(service.cur_svc_status.clone())
        {
            log::error!(
                "failed setting service stopped status after run error: {}",
                set_status_err
            );
        }
        log::error!("node daemon exited with error: {:?}", err);
        return Ok(());
    }

    log::warn!("node daemon exited normally");
    Ok(())
}

pub(crate) fn service_start(matches: ArgMatches) -> windows_service::Result<()> {
    MATCHES.set(matches).unwrap();
    service_dispatcher::start("buckyos", ffi_service_main)
}
