use super::{
    Binding, Boot, ManagerObservation, NormalizedDecl, Registration, Runtime, ServiceError,
    ServiceRoot,
};
use std::path::Path;
use windows_sys::Win32::System::Services::{SERVICE_AUTO_START, SERVICE_DEMAND_START};

#[path = "windows/access.rs"]
mod access;
#[path = "windows/scm.rs"]
pub(crate) mod scm;
#[path = "windows/security.rs"]
mod security;

use access::AccessJournal;
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
        let binding = self.binding_from_command(&config.command).ok();
        let exact = binding.as_ref().is_some_and(|binding| {
            config.command == render_definition(self.root, self.declaration, binding)
                && config.account.eq_ignore_ascii_case(ACCOUNT)
        });
        let registration = if !exact {
            Registration::Conflict
        } else if service.security_is_exact()?
            && self.journal().is_exact(binding.as_ref().unwrap())?
        {
            Registration::Exact
        } else {
            Registration::Broken
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
        let binding = self.binding_from_command(&config.command)?;
        if config.command == render_definition(self.root, self.declaration, &binding) {
            Ok(binding)
        } else {
            Err(ServiceError::invalid("service binding conflicts"))
        }
    }

    pub(super) fn install(&self, binding: &Binding) -> Result<(), ServiceError> {
        let command = render_definition(self.root, self.declaration, binding);
        let service = scm::create(self.declaration, &command, ACCOUNT)?;
        service.configure_security()?;
        self.journal().apply(binding)
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
        self.journal().apply(&binding)
    }

    pub(super) fn enable(&self) -> Result<(), ServiceError> {
        scm::configure(self.declaration)?.enable()
    }

    pub(super) fn disable(&self) -> Result<(), ServiceError> {
        scm::configure(self.declaration)?.disable()
    }

    pub(super) fn rebind(&self, binding: &Binding) -> Result<(), ServiceError> {
        self.journal().apply(binding)?;
        let command = render_definition(self.root, self.declaration, binding);
        scm::rebind(self.declaration)?.set_binding(&command)
    }

    pub(super) fn start(&self) -> Result<(), ServiceError> {
        scm::start(self.declaration)?.start()
    }

    pub(super) fn stop(&self) -> Result<(), ServiceError> {
        scm::stop(self.declaration)?.stop()
    }

    pub(super) fn remove(&self) -> Result<(), ServiceError> {
        let service = scm::removal(self.declaration)?;
        let config = service.config()?;
        let binding = self.binding_from_command(&config.command)?;
        if config.command != render_definition(self.root, self.declaration, &binding) {
            return Err(ServiceError::invalid("service binding conflicts"));
        }
        service.delete()?;
        self.journal().remove()
    }

    fn journal(&self) -> AccessJournal<'a> {
        AccessJournal::new(self)
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
}

pub(super) fn access_plan(
    root: &ServiceRoot,
    declaration: &NormalizedDecl,
    binding: &Binding,
) -> [security::AccessGrant; 2] {
    AccessJournal::new(&WindowsService::new(root, declaration)).plan(binding)
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
