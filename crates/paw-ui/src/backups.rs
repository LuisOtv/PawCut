//! Backups versionados: a cada salvamento, a versão anterior do arquivo fica guardada.

use std::path::PathBuf;

/// Quantas versões anteriores de cada projeto ficam guardadas.
pub const BACKUPS_KEPT: usize = 10;

/// Pasta dos backups de um projeto dentro de `root`: nome do arquivo + um hash do caminho
/// (dois projetos com o mesmo nome em pastas diferentes não se misturam).
pub fn backups_dir_in(root: &std::path::Path, project: &std::path::Path) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    project.hash(&mut h);
    let stem = project
        .file_stem()
        .map_or("projeto".into(), |s| s.to_string_lossy().into_owned());
    root.join(format!("{stem}-{:08x}", h.finish() as u32))
}

/// Copia o arquivo que vai ser sobrescrito para `dir` (nome = segundos desde 1970) e apaga
/// os mais antigos, deixando `keep`. Não faz nada se o arquivo ainda não existe.
pub fn backup_existing(
    path: &std::path::Path,
    dir: &std::path::Path,
    keep: usize,
) -> std::io::Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    std::fs::create_dir_all(dir)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("bak");
    let dest = dir.join(format!("{now:015}.{ext}"));
    std::fs::copy(path, &dest)?;
    for old in list_backups(dir).into_iter().skip(keep) {
        let _ = std::fs::remove_file(old);
    }
    Ok(Some(dest))
}

/// Backups de uma pasta, do mais novo para o mais antigo.
pub fn list_backups(dir: &std::path::Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.parse::<u64>().is_ok())
        })
        .collect();
    files.sort();
    files.reverse();
    files
}

/// Quando o backup foi feito (pelo nome do arquivo).
pub fn backup_time(path: &std::path::Path) -> Option<std::time::SystemTime> {
    let ms: u64 = path.file_stem()?.to_str()?.parse().ok()?;
    Some(std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms))
}

/// "agora", "há 5 min", "há 3 h", "há 2 dias".
pub fn ago(t: std::time::SystemTime) -> String {
    let s = t.elapsed().map_or(0, |d| d.as_secs());
    match s {
        0..60 => "agora".into(),
        60..3600 => format!("há {} min", s / 60),
        3600..86400 => format!("há {} h", s / 3600),
        _ => format!("há {} dia(s)", s / 86400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_only_the_newest_backups() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("filme.pawcut");
        let store = backups_dir_in(&dir.path().join("b"), &project);
        assert_eq!(
            backup_existing(&project, &store, 3).unwrap(),
            None,
            "sem arquivo, sem backup"
        );
        for i in 0..5 {
            std::fs::write(&project, format!("versão {i}")).unwrap();
            backup_existing(&project, &store, 3).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(3));
        }
        let kept = list_backups(&store);
        assert_eq!(kept.len(), 3);
        assert_eq!(std::fs::read_to_string(&kept[0]).unwrap(), "versão 4");
        assert_eq!(std::fs::read_to_string(&kept[2]).unwrap(), "versão 2");
        assert!(backup_time(&kept[0]).is_some());
        assert!(
            store
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("filme-")
        );
    }
}
