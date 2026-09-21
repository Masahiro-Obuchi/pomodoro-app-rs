use super::*;
use crate::{LoadOutcome, StorageLocation, StorageLockError};
use pomodoro_core::{Command, SessionKind, TimerConfig};
use std::fs;

const VALID: &[u8] = include_bytes!("../../../tests/fixtures/state_v1.json");
const OLD_BACKUP: &[u8] = b"previous backup bytes";
const SAVE_STAGES: [SaveStage; 10] = [
    SaveStage::CreateBackupTemp,
    SaveStage::WriteBackupTemp,
    SaveStage::SyncBackupTemp,
    SaveStage::RenameBackup,
    SaveStage::SyncBackupDirectory,
    SaveStage::CreatePrimaryTemp,
    SaveStage::WritePrimaryTemp,
    SaveStage::SyncPrimaryTemp,
    SaveStage::RenamePrimary,
    SaveStage::SyncPrimaryDirectory,
];

fn domain() -> DomainState {
    let mut domain = DomainState::new(TimerConfig::default()).unwrap();
    domain
        .apply(Command::Start(SessionKind::QuickStart), Timestamp(1_000))
        .unwrap();
    domain
}

fn load(location: &StorageLocation) -> WritableStorage {
    match location.clone().lock().unwrap().load().unwrap() {
        LoadOutcome::New(store) | LoadOutcome::Loaded(store) => store,
        LoadOutcome::RecoveryRequired(_) => panic!("unexpected recovery candidate"),
    }
}

mod atomic;
mod failure_matrix;
mod regression;
mod retry;
