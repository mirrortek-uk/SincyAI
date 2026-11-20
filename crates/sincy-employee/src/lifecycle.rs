//! Employee lifecycle state machine (DE §5.1):
//! CREATED → CONFIGURING → READY → RUNNING → PAUSED → STOPPED, plus
//! exceptional WAITING_APPROVAL / DEGRADED / ERROR / ARCHIVED.

use crate::domain::EmployeeStatus;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
#[error("illegal lifecycle transition {from} -> {to}")]
pub struct LifecycleError {
    pub from: EmployeeStatus,
    pub to: EmployeeStatus,
}

/// Legal transitions. Returns Err on illegal ones — never silently coerce.
pub fn transition(
    from: EmployeeStatus,
    to: EmployeeStatus,
) -> Result<EmployeeStatus, LifecycleError> {
    use EmployeeStatus::*;
    let ok = matches!(
        (from, to),
        (Created, Configuring)
            | (Configuring, Ready)
            | (Configuring, WaitingApproval)
            | (Ready, Running)
            | (Running, Paused)
            | (Running, Stopped)
            | (Running, Degraded)
            | (Running, Error)
            | (Paused, Running)
            | (Paused, Stopped)
            | (Degraded, Running)
            | (Degraded, Stopped)
            | (Error, Configuring)
            | (WaitingApproval, Configuring)
            | (WaitingApproval, Stopped)
            | (Stopped, Archived)
            | (Error, Archived)
    );
    if ok {
        Ok(to)
    } else {
        Err(LifecycleError { from, to })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use EmployeeStatus::*;

    #[test]
    fn happy_path_and_illegal() {
        assert_eq!(transition(Created, Configuring).unwrap(), Configuring);
        assert_eq!(transition(Configuring, Ready).unwrap(), Ready);
        assert_eq!(transition(Ready, Running).unwrap(), Running);
        assert_eq!(transition(Running, Paused).unwrap(), Paused);
        assert_eq!(transition(Paused, Running).unwrap(), Running);
        assert!(transition(Created, Running).is_err());
        assert!(transition(Stopped, Running).is_err());
        assert_eq!(transition(Stopped, Archived).unwrap(), Archived);
    }
}
