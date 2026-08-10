use super::super::{Binding, NormalizedDecl, ServiceError, ServiceRoot};
use super::{WindowsService, security};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AccessReceipt {
    schema: u8,
    release: String,
    created: [bool; 2],
}

pub(super) struct AccessJournal<'a> {
    root: &'a ServiceRoot,
    declaration: &'a NormalizedDecl,
}

impl<'a> AccessJournal<'a> {
    pub(super) fn new(service: &WindowsService<'a>) -> Self {
        Self {
            root: service.root,
            declaration: service.declaration,
        }
    }

    pub(super) fn plan(&self, binding: &Binding) -> [security::AccessGrant; 2] {
        security::access_plan(
            binding.release.clone(),
            self.root.declaration(self.declaration),
        )
    }

    pub(super) fn apply(&self, binding: &Binding) -> Result<(), ServiceError> {
        let receipt = self.receipt_for(binding)?;
        let account = self.account();
        for (grant, created) in self.plan(binding).iter().zip(receipt.created) {
            security::apply(grant, &account, created)?;
        }
        Ok(())
    }

    pub(super) fn is_exact(&self, binding: &Binding) -> Result<bool, ServiceError> {
        let Some(receipt) = self.read()? else {
            return Ok(false);
        };
        if receipt.release != binding.relative(self.root).to_string_lossy() {
            return Ok(false);
        }
        let account = self.account();
        self.plan(binding).iter().try_fold(true, |exact, grant| {
            Ok(exact && security::has_access(grant, &account)?)
        })
    }

    pub(super) fn remove(&self) -> Result<(), ServiceError> {
        let receipt = self
            .read()?
            .ok_or_else(|| ServiceError::invalid("access receipt is missing"))?;
        self.revoke(&receipt)
    }

    fn receipt_for(&self, binding: &Binding) -> Result<AccessReceipt, ServiceError> {
        if let Some(receipt) = self.read()? {
            if receipt.release == binding.relative(self.root).to_string_lossy() {
                return Ok(receipt);
            }
            self.revoke(&receipt)?;
            std::fs::remove_file(self.path())
                .map_err(|error| ServiceError::operation("remove stale access receipt", error))?;
        }
        self.create(binding)
    }

    fn create(&self, binding: &Binding) -> Result<AccessReceipt, ServiceError> {
        let plan = self.plan(binding);
        let account = self.account();
        let receipt = AccessReceipt {
            schema: 1,
            release: binding.relative(self.root).to_string_lossy().into_owned(),
            created: [
                !security::has_access(&plan[0], &account)?,
                !security::has_access(&plan[1], &account)?,
            ],
        };
        self.persist(&receipt)?;
        Ok(receipt)
    }

    fn persist(&self, receipt: &AccessReceipt) -> Result<(), ServiceError> {
        use std::io::Write as _;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.path())
            .map_err(|error| ServiceError::operation("create access receipt", error))?;
        let bytes = toml::to_string(receipt)
            .map_err(|error| ServiceError::operation("render access receipt", error))?;
        file.write_all(bytes.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|error| ServiceError::operation("persist access receipt", error))
    }

    fn revoke(&self, receipt: &AccessReceipt) -> Result<(), ServiceError> {
        let binding = Binding::admit(
            self.root,
            self.root.0.join("installs").join(&receipt.release),
            self.declaration,
        )?;
        let account = self.account();
        for (grant, created) in self.plan(&binding).iter().zip(receipt.created) {
            security::revoke(grant, &account, created)?;
        }
        Ok(())
    }

    fn read(&self) -> Result<Option<AccessReceipt>, ServiceError> {
        match std::fs::read_to_string(self.path()) {
            Ok(text) => Self::parse(&text).map(Some),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(ServiceError::operation("read access receipt", error)),
        }
    }

    fn parse(text: &str) -> Result<AccessReceipt, ServiceError> {
        let receipt: AccessReceipt = toml::from_str(text)
            .map_err(|error| ServiceError::operation("parse access receipt", error))?;
        if receipt.schema == 1 {
            Ok(receipt)
        } else {
            Err(ServiceError::invalid("access receipt conflicts"))
        }
    }

    fn account(&self) -> String {
        format!("NT SERVICE\\{}", self.declaration.id.as_str())
    }

    fn path(&self) -> std::path::PathBuf {
        self.root.directory(self.declaration).join("access.receipt")
    }
}
