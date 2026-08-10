use super::{NormalizedDecl, ServiceError, ServiceRoot, platform};
use pulith::Remove;
use std::fmt::{self, Display};

pub struct Service {
    root: ServiceRoot,
    declaration: NormalizedDecl,
}

impl Service {
    pub fn new(root: ServiceRoot, declaration: NormalizedDecl) -> Self {
        Self { root, declaration }
    }
    pub fn status(self) -> Result<Observation, ServiceError> {
        self.observe()
    }

    pub fn install(self) -> Result<Change, ServiceError> {
        let before = self.observe()?;
        if before.definition == Definition::Exact && before.registration == Registration::Exact {
            return Ok(Change::unchanged(before));
        }
        if before.definition == Definition::Exact && before.registration == Registration::Broken {
            let binding = self.platform().binding()?;
            self.root.admit_exposure(&binding, &self.declaration)?;
            self.platform().repair()?;
            return self.changed();
        }
        if !matches!(before.definition, Definition::Missing | Definition::Exact)
            || before.registration != Registration::Missing
        {
            return Err(state_error("registration", before.registration));
        }
        let binding = self.root.active_binding(&self.declaration)?;
        self.root.admit_exposure(&binding, &self.declaration)?;
        if before.definition == Definition::Missing {
            self.root.publish(&self.declaration)?;
        }
        self.root.admit_exposure(&binding, &self.declaration)?;
        self.platform().install(&binding)?;
        self.changed()
    }

    pub fn enable(self) -> Result<Change, ServiceError> {
        let before = self.exact()?;
        if before.boot == Boot::Enabled {
            return Ok(Change::unchanged(before));
        }
        if before.boot != Boot::Disabled {
            return Err(ServiceError::invalid("boot state conflicts"));
        }
        let binding = self.platform().binding()?;
        self.root.admit_exposure(&binding, &self.declaration)?;
        self.platform().enable()?;
        self.changed()
    }

    pub fn rebind(self) -> Result<Change, ServiceError> {
        let before = self.exact()?;
        if before.runtime != Runtime::Stopped {
            return Err(ServiceError::invalid("stop service before rebinding"));
        }
        let binding = self.root.active_binding(&self.declaration)?;
        self.root.admit_exposure(&binding, &self.declaration)?;
        self.platform().rebind(&binding)?;
        self.changed()
    }

    pub fn disable(self) -> Result<Change, ServiceError> {
        let before = self.exact()?;
        if before.boot == Boot::Disabled {
            return Ok(Change::unchanged(before));
        }
        if before.runtime != Runtime::Stopped {
            return Err(ServiceError::invalid("stop service before disabling"));
        }
        self.platform().disable()?;
        self.changed()
    }

    pub fn start(self) -> Result<Change, ServiceError> {
        let before = self.exact()?;
        if before.runtime == Runtime::Running {
            return Ok(Change::unchanged(before));
        }
        if before.runtime != Runtime::Stopped {
            return Err(ServiceError::invalid("runtime is not stopped"));
        }
        let binding = self.platform().binding()?;
        self.root.admit_exposure(&binding, &self.declaration)?;
        self.platform().start()?;
        self.changed()
    }

    pub fn stop(self) -> Result<Change, ServiceError> {
        let before = self.exact()?;
        if before.runtime == Runtime::Stopped {
            return Ok(Change::unchanged(before));
        }
        if !matches!(before.runtime, Runtime::Running | Runtime::Failed) {
            return Err(ServiceError::invalid(
                "runtime transition already in progress",
            ));
        }
        self.platform().stop()?;
        self.changed()
    }

    pub fn restart(self) -> Result<Change, ServiceError> {
        let before = self.exact()?;
        let binding = self.platform().binding()?;
        self.root.admit_exposure(&binding, &self.declaration)?;
        match before.runtime {
            Runtime::Running | Runtime::Failed => self.platform().stop()?,
            Runtime::Stopped => {}
            _ => {
                return Err(ServiceError::invalid(
                    "runtime transition already in progress",
                ));
            }
        }
        self.platform().start()?;
        self.changed()
    }

    pub fn remove(self) -> Result<Change, ServiceError> {
        let before = self.observe()?;
        if before.registration == Registration::Missing {
            return Ok(Change::unchanged(before));
        }
        if before.runtime != Runtime::Stopped || before.boot != Boot::Disabled {
            return Err(ServiceError::invalid(
                "remove requires stopped and disabled",
            ));
        }
        self.platform().remove()?;
        self.root.recheck()?;
        let target = ServiceError::effect(pulith::local::LocalTarget::new(
            self.root.directory(&self.declaration),
        ))?;
        ServiceError::effect(target.remove())?;
        self.changed()
    }

    fn exact(&self) -> Result<Observation, ServiceError> {
        let observation = self.observe()?;
        if observation.registration == Registration::Exact {
            Ok(observation)
        } else {
            Err(state_error("registration", observation.registration))
        }
    }

    fn observe(&self) -> Result<Observation, ServiceError> {
        let definition = self.root.observe(&self.declaration)?;
        let manager = self.platform().observe()?;
        Ok(Observation {
            definition,
            registration: manager.registration,
            boot: manager.boot,
            runtime: manager.runtime,
        })
    }

    fn changed(&self) -> Result<Change, ServiceError> {
        Ok(Change {
            changed: true,
            observation: self.observe()?,
        })
    }

    fn platform(&self) -> platform::PlatformService<'_> {
        platform::PlatformService::new(&self.root, &self.declaration)
    }
}

fn state_error(label: &str, value: impl fmt::Debug) -> ServiceError {
    ServiceError::invalid(format!("{label} is {}", word(value)))
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[rustfmt::skip]
pub enum Definition { Missing, Exact, Conflict, Broken }
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[rustfmt::skip]
pub enum Registration { Missing, Exact, Conflict, Broken, Removing }
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[rustfmt::skip]
pub enum Boot { Disabled, Enabled, Conflict }
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[rustfmt::skip]
pub enum Runtime { Stopped, Starting, Running, Stopping, Failed }
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Observation {
    pub definition: Definition,
    pub registration: Registration,
    pub boot: Boot,
    pub runtime: Runtime,
}
pub(crate) struct ManagerObservation {
    pub registration: Registration,
    pub boot: Boot,
    pub runtime: Runtime,
}
pub struct Change {
    pub changed: bool,
    pub observation: Observation,
}

impl Change {
    fn unchanged(observation: Observation) -> Self {
        Self {
            changed: false,
            observation,
        }
    }
}
impl Display for Observation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "definition={} registration={} boot={} runtime={}",
            word(self.definition),
            word(self.registration),
            word(self.boot),
            word(self.runtime)
        )
    }
}
impl Display for Change {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "changed={} {}", self.changed, self.observation)
    }
}
pub(super) fn word(value: impl fmt::Debug) -> String {
    format!("{value:?}").to_ascii_lowercase()
}
