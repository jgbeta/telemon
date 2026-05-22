use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::watch;
use windows_service::service::{
    ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceInfo,
    ServiceStartType, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_dispatcher;
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

use crate::cli::ServiceCommand;
use crate::runtime;
use telemon_core::config::AppConfig;
use telemon_core::logging;

const SERVICE_NAME: &str = "TelemonExporter";
const SERVICE_DISPLAY_NAME: &str = "Telemon Exporter";

static SERVICE_CONFIG_PATH: OnceLock<PathBuf> = OnceLock::new();

pub async fn handle(command: ServiceCommand) -> Result<()> {
    match command {
        ServiceCommand::Install(args) => install(args.config),
        ServiceCommand::Uninstall => uninstall(),
        ServiceCommand::Start => start(),
        ServiceCommand::Stop => stop(),
        ServiceCommand::Status => status(),
        ServiceCommand::Run(args) => run_service_command(args.config),
    }
}

fn run_service_command(config_path: PathBuf) -> Result<()> {
    let _ = SERVICE_CONFIG_PATH.set(config_path.clone());
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
        .or_else(|_| run_service(config_path))
        .context("failed to start Windows service dispatcher")
}

fn install(config_path: PathBuf) -> Result<()> {
    let manager = service_manager(ServiceManagerAccess::CREATE_SERVICE)?;
    let exe_path = std::env::current_exe().context("failed to locate current executable")?;
    let info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from(SERVICE_DISPLAY_NAME),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: exe_path,
        launch_arguments: vec![
            OsString::from("service"),
            OsString::from("run"),
            OsString::from("--config"),
            OsString::from(config_path),
        ],
        dependencies: vec![],
        account_name: Some(OsString::from("NT AUTHORITY\\LocalService")),
        account_password: None,
    };

    manager
        .create_service(&info, ServiceAccess::CHANGE_CONFIG | ServiceAccess::START)
        .context("failed to create Windows service")?;
    println!("installed {SERVICE_NAME}");
    Ok(())
}

fn uninstall() -> Result<()> {
    let manager = service_manager(ServiceManagerAccess::CONNECT)?;
    let service = manager
        .open_service(SERVICE_NAME, ServiceAccess::STOP | ServiceAccess::DELETE)
        .context("failed to open Windows service")?;
    let _ = service.stop();
    service
        .delete()
        .context("failed to delete Windows service")?;
    println!("uninstalled {SERVICE_NAME}");
    Ok(())
}

fn start() -> Result<()> {
    let manager = service_manager(ServiceManagerAccess::CONNECT)?;
    let service = manager
        .open_service(SERVICE_NAME, ServiceAccess::START)
        .context("failed to open Windows service")?;
    service
        .start::<OsString>(&[])
        .context("failed to start Windows service")?;
    println!("started {SERVICE_NAME}");
    Ok(())
}

fn stop() -> Result<()> {
    let manager = service_manager(ServiceManagerAccess::CONNECT)?;
    let service = manager
        .open_service(SERVICE_NAME, ServiceAccess::STOP)
        .context("failed to open Windows service")?;
    service.stop().context("failed to stop Windows service")?;
    println!("stopped {SERVICE_NAME}");
    Ok(())
}

fn status() -> Result<()> {
    let manager = service_manager(ServiceManagerAccess::CONNECT)?;
    let service = manager
        .open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS)
        .context("failed to open Windows service")?;
    let status = service
        .query_status()
        .context("failed to query service status")?;
    println!("{SERVICE_NAME}: {:?}", status.current_state);
    Ok(())
}

fn service_manager(access: ServiceManagerAccess) -> Result<ServiceManager> {
    ServiceManager::local_computer(None::<&str>, access).context("failed to open service manager")
}

windows_service::define_windows_service!(ffi_service_main, service_main);

fn service_main(arguments: Vec<OsString>) {
    if let Err(error) = service_main_inner(arguments) {
        eprintln!("Windows service failed: {error:#}");
    }
}

fn service_main_inner(arguments: Vec<OsString>) -> Result<()> {
    let config_path = config_path_from_arguments(&arguments)
        .or_else(|| SERVICE_CONFIG_PATH.get().cloned())
        .context("missing --config for Windows service run")?;

    run_service(config_path)
}

fn config_path_from_arguments(arguments: &[OsString]) -> Option<PathBuf> {
    arguments
        .windows(2)
        .find(|window| window[0] == "--config")
        .map(|window| PathBuf::from(&window[1]))
        .or_else(|| {
            arguments
                .iter()
                .position(|value| value == "--config")
                .and_then(|index| arguments.get(index + 1))
                .map(PathBuf::from)
        })
}

fn run_service(config_path: PathBuf) -> Result<()> {
    let config = AppConfig::load_from_path(&config_path)?;
    logging::init(&config.logging.level)?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let control_shutdown_tx = shutdown_tx.clone();

    let status_handle = service_control_handler::register(SERVICE_NAME, move |control| {
        if control == ServiceControl::Stop {
            let _ = control_shutdown_tx.send(true);
            ServiceControlHandlerResult::NoError
        } else {
            ServiceControlHandlerResult::NotImplemented
        }
    })
    .context("failed to register service control handler")?;

    status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP,
        exit_code: windows_service::service::ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::from_secs(10),
        process_id: None,
    })?;

    let runtime = tokio::runtime::Runtime::new().context("failed to create Tokio runtime")?;
    let result = runtime.block_on(runtime::run_with_shutdown(config, shutdown_tx, shutdown_rx));

    status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: windows_service::service::ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::from_secs(0),
        process_id: None,
    })?;

    result
}
