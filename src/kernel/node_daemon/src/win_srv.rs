use std::cell::OnceCell;
use std::ffi::OsString;
use std::process::exit;
use std::sync::OnceLock;
use std::time::Duration;
use clap::{Arg, ArgMatches, Command};
use lazy_static::lazy_static;
use windows_service::{define_windows_service, service_control_handler, service_dispatcher, Error};
use windows_service::service::{ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType};
use windows_service::service_control_handler::{ServiceControlHandlerResult, ServiceStatusHandle};
use crate::run;

define_windows_service!(ffi_service_main, service_main);

struct WinService {
    status_handle: Option<ServiceStatusHandle>,
    cur_svc_status: ServiceStatus,
}

lazy_static! {
    static ref SERVICE: std::sync::RwLock<WinService> = std::sync::RwLock::new(WinService{
        status_handle: None,
        cur_svc_status: ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: ServiceState::StartPending,
            controls_accepted: ServiceControlAccept::empty(),
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
    }});
}

static MATCHES: OnceLock<ArgMatches> = OnceLock::new();

pub(crate) fn service_main(_arguments: Vec<OsString>) -> windows_service::Result<()> {
    let status_handle = service_control_handler::register("buckyos", move |event| {
        match event {
            ServiceControl::Stop => {
                let mut service = SERVICE.write().unwrap();

                service.cur_svc_status.current_state = ServiceState::Stopped;
                service.cur_svc_status.controls_accepted = ServiceControlAccept::empty();

                service.status_handle.unwrap().set_service_status(service.cur_svc_status.clone());

                //exit(0);
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => {
                let mut status = SERVICE.read().unwrap().cur_svc_status.clone();
                status.process_id = Some(std::process::id());
                SERVICE.read().unwrap().status_handle.unwrap().set_service_status(status);
                ServiceControlHandlerResult::NoError
            }
            _ => {
                ServiceControlHandlerResult::NotImplemented
            }
        }
    })?;

    {
        let mut service = SERVICE.write().unwrap();

        service.cur_svc_status.current_state = ServiceState::Running;
        service.cur_svc_status.controls_accepted = ServiceControlAccept::STOP;
        service.status_handle = Some(status_handle);

        service.status_handle.unwrap().set_service_status(service.cur_svc_status.clone());

    }

    let matches = MATCHES.get().unwrap().clone();

    run::run(matches);
    log::warn!("service exited!!");
    Ok(())
}

pub(crate) fn service_start(matches: ArgMatches) -> windows_service::Result<()> {
    MATCHES.set(matches).unwrap();
    service_dispatcher::start("buckyos", ffi_service_main)
}