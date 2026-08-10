use super::super::{Binding, NormalizedDecl, ServiceError, ServiceRoot};
use super::{WindowsService, security};
use pulith::local::{RecordEdit, RecordLimit, RecordObservation, RecordStore};
use serde::{Deserialize, Serialize};
use std::io::Cursor;

const RECEIPT: &str = "access.receipt";
const INTENT: &str = "rebind.intent";
const LIMIT: u64 = 16 * 1024;

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
enum Ownership {
    Created,
    Preexisting,
}

#[derive(Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct GrantReceipt {
    path: String,
    mask: u32,
    inheritance: u32,
    ownership: Ownership,
}

#[derive(Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct AccessReceipt {
    schema: u8,
    release: String,
    grants: [GrantReceipt; 2],
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RebindIntent {
    schema: u8,
    from: AccessReceipt,
    to: AccessReceipt,
}

pub(super) struct AccessState<'a> {
    root: &'a ServiceRoot,
    declaration: &'a NormalizedDecl,
}

pub(super) struct RebindEdit<'a> {
    state: AccessState<'a>,
    edit: RecordEdit,
    intent: RebindIntent,
}

impl<'a> AccessState<'a> {
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
        let mut edit = self.store()?.edit().map_err(record)?;
        if present(&edit, INTENT)?.is_some() {
            return Err(ServiceError::invalid("rebind intent is pending"));
        }
        let receipt = match present(&edit, RECEIPT)? {
            Some(bytes) => {
                let receipt = self.parse_receipt(&bytes)?;
                if !self.matches(&receipt, binding)? {
                    return Err(ServiceError::invalid("access receipt conflicts"));
                }
                receipt
            }
            None => {
                let receipt = self.receipt(binding)?;
                create(&mut edit, RECEIPT, &receipt)?;
                receipt
            }
        };
        self.ensure(&receipt)
    }

    pub(super) fn is_exact(&self, binding: &Binding) -> Result<bool, ServiceError> {
        let Some(bytes) = self.read(RECEIPT)? else {
            return Ok(false);
        };
        let receipt = self.parse_receipt(&bytes)?;
        if !self.matches(&receipt, binding)? {
            return Ok(false);
        }
        let account = self.account();
        self.plan(binding).iter().try_fold(true, |exact, grant| {
            Ok(exact && security::has_access(grant, &account)?)
        })
    }

    pub(super) fn begin_rebind(
        &self,
        observed: &Binding,
        target: &Binding,
    ) -> Result<RebindEdit<'a>, ServiceError> {
        let mut edit = self.store()?.edit().map_err(record)?;
        let stable = self.parse_required(&edit, RECEIPT)?;
        let intent = match present(&edit, INTENT)? {
            Some(bytes) => {
                let intent: RebindIntent = decode(INTENT, &bytes)?;
                self.validate_intent(&intent, &stable, observed, target)?;
                intent
            }
            None => {
                if !self.matches(&stable, observed)? {
                    return Err(ServiceError::invalid(
                        "stable access receipt conflicts with manager binding",
                    ));
                }
                let intent = RebindIntent {
                    schema: 1,
                    from: stable,
                    to: self.receipt(target)?,
                };
                create(&mut edit, INTENT, &intent)?;
                intent
            }
        };
        Ok(RebindEdit {
            state: Self {
                root: self.root,
                declaration: self.declaration,
            },
            edit,
            intent,
        })
    }

    pub(super) fn cleanup_removed(&self) -> Result<(), ServiceError> {
        let mut edit = self.store()?.edit().map_err(record)?;
        let stable = present(&edit, RECEIPT)?
            .map(|bytes| self.parse_receipt(&bytes))
            .transpose()?;
        let intent: Option<RebindIntent> = present(&edit, INTENT)?
            .map(|bytes| decode(INTENT, &bytes))
            .transpose()?;
        if let Some(intent) = &intent {
            self.validate_receipt(&intent.from)?;
            self.validate_receipt(&intent.to)?;
        }
        let account = self.account();
        if let Some(receipt) = stable.as_ref() {
            self.revoke(receipt, &account, &[])?;
        }
        if let Some(intent) = intent.as_ref() {
            let stable_grants = stable
                .as_ref()
                .map(|receipt| self.plan(&self.binding(receipt).expect("validated receipt")));
            let shared = stable_grants
                .as_ref()
                .map(<[_; 2]>::as_slice)
                .unwrap_or(&[]);
            self.revoke(&intent.from, &account, shared)?;
            let from = self.plan(&self.binding(&intent.from)?);
            self.revoke(&intent.to, &account, &from)?;
        }
        if stable.is_some() {
            edit.remove(RECEIPT).map_err(record)?;
        }
        if intent.is_some() {
            edit.remove(INTENT).map_err(record)?;
        }
        Ok(())
    }

    fn revoke(
        &self,
        receipt: &AccessReceipt,
        account: &str,
        skip: &[security::AccessGrant],
    ) -> Result<(), ServiceError> {
        let binding = self.binding(receipt)?;
        for (grant, fact) in self.plan(&binding).iter().zip(&receipt.grants) {
            if !skip.iter().any(|candidate| same_grant(grant, candidate)) {
                security::revoke(grant, account, fact.ownership == Ownership::Created)?;
            }
        }
        Ok(())
    }

    fn validate_intent(
        &self,
        intent: &RebindIntent,
        stable: &AccessReceipt,
        observed: &Binding,
        target: &Binding,
    ) -> Result<(), ServiceError> {
        if intent.schema != 1
            || intent.from != *stable
            || !self.matches(&intent.to, target)?
            || (!self.matches(&intent.from, observed)? && !self.matches(&intent.to, observed)?)
        {
            return Err(ServiceError::invalid("rebind intent conflicts"));
        }
        self.validate_receipt(&intent.from)?;
        self.validate_receipt(&intent.to)
    }

    fn receipt(&self, binding: &Binding) -> Result<AccessReceipt, ServiceError> {
        let account = self.account();
        let plan = self.plan(binding);
        let grants = [
            self.grant(&plan[0], &account)?,
            self.grant(&plan[1], &account)?,
        ];
        Ok(AccessReceipt {
            schema: 1,
            release: relative(self.root, &binding.release)?,
            grants,
        })
    }

    fn grant(
        &self,
        grant: &security::AccessGrant,
        account: &str,
    ) -> Result<GrantReceipt, ServiceError> {
        Ok(GrantReceipt {
            path: relative(self.root, &grant.path)?,
            mask: grant.mask,
            inheritance: grant.inheritance,
            ownership: if security::has_access(grant, account)? {
                Ownership::Preexisting
            } else {
                Ownership::Created
            },
        })
    }

    fn ensure(&self, receipt: &AccessReceipt) -> Result<(), ServiceError> {
        let binding = self.binding(receipt)?;
        let account = self.account();
        for (grant, fact) in self.plan(&binding).iter().zip(&receipt.grants) {
            security::apply(grant, &account, fact.ownership == Ownership::Created)?;
        }
        Ok(())
    }

    fn parse_receipt(&self, bytes: &[u8]) -> Result<AccessReceipt, ServiceError> {
        let receipt: AccessReceipt = decode(RECEIPT, bytes)?;
        self.validate_receipt(&receipt)?;
        Ok(receipt)
    }

    fn validate_receipt(&self, receipt: &AccessReceipt) -> Result<(), ServiceError> {
        let binding = self.binding(receipt)?;
        let plan = self.plan(&binding);
        if receipt.schema != 1
            || receipt.grants[0].path == receipt.grants[1].path
            || !plan.iter().zip(&receipt.grants).all(|(grant, fact)| {
                fact.path == relative(self.root, &grant.path).unwrap_or_default()
                    && fact.mask == grant.mask
                    && fact.inheritance == grant.inheritance
            })
        {
            Err(ServiceError::invalid("access receipt conflicts"))
        } else {
            Ok(())
        }
    }

    fn binding(&self, receipt: &AccessReceipt) -> Result<Binding, ServiceError> {
        Binding::admit(
            self.root,
            self.root.0.join(&receipt.release),
            self.declaration,
        )
    }

    fn matches(&self, receipt: &AccessReceipt, binding: &Binding) -> Result<bool, ServiceError> {
        Ok(receipt.release == relative(self.root, &binding.release)?)
    }

    fn parse_required(&self, edit: &RecordEdit, name: &str) -> Result<AccessReceipt, ServiceError> {
        let bytes = present(edit, name)?
            .ok_or_else(|| ServiceError::invalid("access receipt is missing"))?;
        self.parse_receipt(&bytes)
    }

    fn read(&self, name: &str) -> Result<Option<Vec<u8>>, ServiceError> {
        let directory = self.root.directory(self.declaration).join("state");
        if !directory.is_dir() {
            return Ok(None);
        }
        let store = RecordStore::new(directory).map_err(record)?;
        match store.inspect(name, limit()).map_err(record)?.0 {
            RecordObservation::Missing => Ok(None),
            RecordObservation::Present(bytes) => Ok(Some(bytes)),
        }
    }

    fn store(&self) -> Result<RecordStore, ServiceError> {
        RecordStore::new(self.root.directory(self.declaration).join("state")).map_err(record)
    }

    fn account(&self) -> String {
        format!("NT SERVICE\\{}", self.declaration.id.as_str())
    }
}

impl RebindEdit<'_> {
    pub(super) fn ensure_target(&self) -> Result<(), ServiceError> {
        self.state.ensure(&self.intent.to)
    }

    pub(super) fn needs_switch(&self, binding: &Binding) -> Result<bool, ServiceError> {
        if self.state.matches(&self.intent.from, binding)? {
            Ok(true)
        } else if self.state.matches(&self.intent.to, binding)? {
            Ok(false)
        } else {
            Err(ServiceError::invalid(
                "manager binding conflicts with rebind intent",
            ))
        }
    }

    pub(super) fn finish(mut self, binding: &Binding) -> Result<(), ServiceError> {
        if !self.state.matches(&self.intent.to, binding)? {
            return Err(ServiceError::invalid(
                "manager did not accept target binding",
            ));
        }
        self.ensure_target()?;
        let from = self.state.binding(&self.intent.from)?;
        let to = self.state.binding(&self.intent.to)?;
        let target = self.state.plan(&to);
        let account = self.state.account();
        for (grant, fact) in self.state.plan(&from).iter().zip(&self.intent.from.grants) {
            let shared = target.iter().any(|candidate| same_grant(grant, candidate));
            if !shared {
                security::revoke(grant, &account, fact.ownership == Ownership::Created)?;
            }
        }
        replace(&mut self.edit, RECEIPT, &self.intent.to)?;
        self.edit.remove(INTENT).map_err(record)?;
        Ok(())
    }
}

fn present(edit: &RecordEdit, name: &str) -> Result<Option<Vec<u8>>, ServiceError> {
    match edit.inspect(name, limit()).map_err(record)?.0 {
        RecordObservation::Missing => Ok(None),
        RecordObservation::Present(bytes) => Ok(Some(bytes)),
    }
}

fn create(edit: &mut RecordEdit, name: &str, value: &impl Serialize) -> Result<(), ServiceError> {
    let bytes = toml::to_string(value)
        .map_err(|error| ServiceError::operation("render recovery record", error))?;
    edit.create_from(name, limit(), Cursor::new(bytes))
        .map_err(record)?;
    Ok(())
}

fn replace(edit: &mut RecordEdit, name: &str, value: &impl Serialize) -> Result<(), ServiceError> {
    let bytes = toml::to_string(value)
        .map_err(|error| ServiceError::operation("render recovery record", error))?;
    edit.replace_from(name, limit(), Cursor::new(bytes))
        .map_err(record)?;
    Ok(())
}

fn decode<T: for<'de> Deserialize<'de>>(name: &str, bytes: &[u8]) -> Result<T, ServiceError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| ServiceError::operation("decode recovery record", error))?;
    toml::from_str(text).map_err(|error| {
        ServiceError::operation("parse recovery record", format_args!("{name}: {error}"))
    })
}

fn relative(root: &ServiceRoot, path: &std::path::Path) -> Result<String, ServiceError> {
    path.strip_prefix(&root.0)
        .ok()
        .and_then(std::path::Path::to_str)
        .filter(|path| !path.is_empty() && !path.contains(".."))
        .map(str::to_owned)
        .ok_or_else(|| ServiceError::invalid("recovery record path conflicts"))
}

fn same_grant(left: &security::AccessGrant, right: &security::AccessGrant) -> bool {
    left.path == right.path && left.mask == right.mask && left.inheritance == right.inheritance
}

fn limit() -> RecordLimit {
    RecordLimit::new(LIMIT).expect("positive recovery record limit")
}
fn record(error: pulith::local::RecordError) -> ServiceError {
    ServiceError::operation("access record", error)
}

#[cfg(test)]
#[path = "../../../../tests/examples/toolhost/service/windows/access.rs"]
mod tests;
