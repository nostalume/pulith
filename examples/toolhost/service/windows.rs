use super::{
    AcceptedEffect, Binding, Boot, ManagerObservation, NormalizedDecl, Observation, Registration,
    Runtime, ServiceError, ServiceRoot,
};
use std::path::Path;
use windows_sys::Win32::System::Services::{SERVICE_AUTO_START, SERVICE_DEMAND_START};

#[path = "windows/access.rs"]
mod access;
#[path = "windows/scm.rs"]
pub(crate) mod scm;
#[path = "windows/security.rs"]
mod security;

use access::AccessState;
pub(super) use security::{secure_ancestor, secure_input, secure_leaf};

pub(super) const ACCOUNT: &str = "NT AUTHORITY\\LocalService";

pub(super) struct WindowsService<'a> {
    pub(super) root: &'a ServiceRoot,
    pub(super) declaration: &'a NormalizedDecl,
}

pub(super) type PlatformService<'a> = WindowsService<'a>;

impl<'a> WindowsService<'a> {
    pub(super) fn new(root: &'a ServiceRoot, declaration: &'a NormalizedDecl) -> Self {
        Self { root, declaration }
    }

    pub(super) fn observe(&self) -> Result<ManagerObservation, ServiceError> {
        let service = match scm::observe(self.declaration)? {
            scm::OpenedService::Missing => return Ok(missing()),
            scm::OpenedService::Removing => return Ok(removing()),
            scm::OpenedService::Present(service) => service,
        };
        let config = service.config()?;
        let binding = self.admit_binding(&config).ok();
        let registration = match binding {
            None => Registration::Conflict,
            Some(binding) if service.security_is_exact()? && self.state().is_exact(&binding)? => {
                Registration::Exact
            }
            Some(_) => Registration::Broken,
        };
        Ok(ManagerObservation {
            registration,
            boot: match config.start_type {
                SERVICE_AUTO_START => Boot::Enabled,
                SERVICE_DEMAND_START => Boot::Disabled,
                _ => Boot::Conflict,
            },
            runtime: service.runtime()?,
        })
    }

    pub(super) fn binding(&self) -> Result<Binding, ServiceError> {
        let config = scm::binding(self.declaration)?.config()?;
        self.admit_binding(&config)
    }

    pub(super) fn install(&self, binding: &Binding) -> Result<(), ServiceError> {
        let command = render_definition(self.root, self.declaration, binding);
        let service = scm::create(self.declaration, &command, ACCOUNT)?;
        service.configure_security()?;
        self.state().apply(binding)
    }

    pub(super) fn repair(&self) -> Result<(), ServiceError> {
        let service = scm::repair(self.declaration)?;
        let config = service.config()?;
        let binding = self.binding_from_command(&config.command)?;
        if config.command != render_definition(self.root, self.declaration, &binding)
            || !config.account.eq_ignore_ascii_case(ACCOUNT)
        {
            return Err(ServiceError::invalid("service registration conflicts"));
        }
        service.configure_security()?;
        self.state().apply(&binding)
    }

    pub(super) fn enable(&self) -> Result<(), ServiceError> {
        scm::configure(self.declaration)?.enable()
    }

    pub(super) fn disable(&self) -> Result<(), ServiceError> {
        scm::configure(self.declaration)?.disable()
    }

    pub(super) fn rebind(&self, binding: &Binding) -> Result<(), ServiceError> {
        let service = scm::rebind(self.declaration)?;
        let config = service.config()?;
        let observed = self.admit_rebind(&config, service.security_is_exact()?)?;
        let transition = self.state().begin_rebind(&observed, binding)?;
        transition.ensure_target()?;
        let config = service.config()?;
        let current = self.admit_rebind(&config, service.security_is_exact()?)?;
        if transition.needs_switch(&current)? {
            let command = render_definition(self.root, self.declaration, binding);
            service.set_binding(&command)?;
        }
        let config = service.config()?;
        let observed = self.admit_rebind(&config, service.security_is_exact()?)?;
        transition.finish(&observed)
    }

    pub(super) fn start(&self) -> Result<(), ServiceError> {
        scm::start(self.declaration)?.start()
    }

    pub(super) fn stop(&self) -> Result<(), ServiceError> {
        scm::stop(self.declaration)?.stop()
    }

    pub(super) fn remove(&self) -> Result<(), ServiceError> {
        match scm::observe_for_removal(self.declaration)? {
            scm::Removal::Missing => {}
            scm::Removal::Removing => self.await_removal()?,
            scm::Removal::Present(service) => {
                let config = service.config()?;
                let binding = self.binding_from_command(&config.command)?;
                if config.command != render_definition(self.root, self.declaration, &binding) {
                    return Err(ServiceError::invalid("service binding conflicts"));
                }
                service.delete()?;
                self.await_removal()?;
            }
        }
        self.state().cleanup_removed()
    }

    fn await_removal(&self) -> Result<(), ServiceError> {
        let definition = self.root.observe(self.declaration)?;
        scm::await_missing(self.declaration).map_err(|error| {
            let manager = removing();
            ServiceError::partial(
                AcceptedEffect::DeletionRequested,
                Observation {
                    definition,
                    registration: manager.registration,
                    boot: manager.boot,
                    runtime: manager.runtime,
                },
                "await service deletion",
                error,
            )
        })
    }

    fn state(&self) -> AccessState<'a> {
        AccessState::new(self)
    }

    fn binding_from_command(&self, command: &str) -> Result<Binding, ServiceError> {
        let suffix = format!(
            "\" \"{}\"",
            self.root.declaration(self.declaration).display()
        );
        let host = command
            .strip_prefix('"')
            .and_then(|command| command.strip_suffix(&suffix))
            .ok_or_else(|| ServiceError::invalid("service command shape conflicts"))?;
        Binding::from_host(self.root, Path::new(host), self.declaration)
    }

    fn admit_binding(&self, config: &scm::ServiceConfig) -> Result<Binding, ServiceError> {
        let binding = self.binding_from_command(&config.command)?;
        if config.command == render_definition(self.root, self.declaration, &binding)
            && config.account.eq_ignore_ascii_case(ACCOUNT)
        {
            Ok(binding)
        } else {
            Err(ServiceError::invalid("service registration conflicts"))
        }
    }

    pub(super) fn admit_rebind(
        &self,
        config: &scm::ServiceConfig,
        security_exact: bool,
    ) -> Result<Binding, ServiceError> {
        let binding = self.admit_binding(config)?;
        security_exact
            .then_some(binding)
            .ok_or_else(|| ServiceError::invalid("service security conflicts"))
    }
}

pub(super) fn access_plan(
    root: &ServiceRoot,
    declaration: &NormalizedDecl,
    binding: &Binding,
) -> [security::AccessGrant; 2] {
    AccessState::new(&WindowsService::new(root, declaration)).plan(binding)
}

pub(super) fn render_definition(
    root: &ServiceRoot,
    declaration: &NormalizedDecl,
    binding: &Binding,
) -> String {
    let host = binding.host(declaration);
    format!(
        "\"{}\" \"{}\"",
        host.display(),
        root.declaration(declaration).display()
    )
}

fn missing() -> ManagerObservation {
    ManagerObservation {
        registration: Registration::Missing,
        boot: Boot::Disabled,
        runtime: Runtime::Stopped,
    }
}

fn removing() -> ManagerObservation {
    ManagerObservation {
        registration: Registration::Removing,
        boot: Boot::Conflict,
        runtime: Runtime::Stopping,
    }
}
