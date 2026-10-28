use crate::{
    Error, ErrorDetail, ErrorKind, Fs, Metadata, MountPoint, MountTable, Platform, Reporter,
    discovery::Discovery,
};
use alloc::{vec, vec::Vec};
use binn_path::BytePath;
use binn_spec::TrashLayout;

const NO_TRASH_CANDIDATE_OPERATION: &str =
    "find trash (no usable home or top-directory candidate; set an absolute HOME or XDG_DATA_HOME)";

pub(crate) struct PutSource<D> {
    pub(crate) parent: D,
    pub(crate) name: Vec<u8>,
    pub(crate) original: BytePath,
    pub(crate) metadata: Metadata,
}

pub(crate) struct Candidate {
    pub(crate) layout: TrashLayout,
    pub(crate) copy_on_cross_device: bool,
}

pub(crate) struct CandidateSelection;

impl CandidateSelection {
    pub(crate) fn protect_trash_relationship(
        source: &BytePath,
        trash_root: &BytePath,
    ) -> Result<(), Error> {
        if source.starts_with(trash_root) || trash_root.starts_with(source) {
            return Err(Error::with_detail(
                ErrorKind::Unsafe,
                "trash",
                Some(source.clone()),
                ErrorDetail::SourceOverlapsTrash,
            ));
        }
        Ok(())
    }

    pub(crate) fn is_mountpoint<P: Platform>(
        platform: &P,
        source: &PutSource<<<P as Platform>::Fs as Fs>::Dir>,
    ) -> Result<bool, Error> {
        let parent_meta = platform.fs().dir_metadata(&source.parent)?;
        let parent_path = platform.fs().real_path(&source.parent)?;
        let mount = platform
            .mounts()
            .mount_of(&source.metadata, &source.original)?;
        let parent_mount = platform.mounts().mount_of(&parent_meta, &parent_path)?;
        if let (Some(source_id), Some(parent_id)) = (mount.mount_id(), parent_mount.mount_id()) {
            return Ok(source_id != parent_id);
        }
        Ok(source.original == *mount.mount_point())
    }

    pub(crate) fn candidates<P: Platform>(
        platform: &P,
        custom: Option<&BytePath>,
        source_mount: &MountPoint,
        reporter: &mut dyn Reporter,
    ) -> Result<Vec<Candidate>, Error> {
        if let Some(root) = custom {
            let layout = Discovery::custom_layout(platform, root, "select trash")?;
            return Ok(vec![Candidate {
                layout,
                copy_on_cross_device: true,
            }]);
        }
        let home = Discovery::home_layout(platform.identity());
        if let Some(home) = &home
            && let Ok(home_trash) = Discovery::ensure(platform.fs(), home.clone())
        {
            let home_mount = platform
                .mounts()
                .mount_of(&home_trash.metadata, home.root())?;
            if home_mount.same_mount(source_mount) {
                return Ok(vec![Candidate {
                    layout: home.clone(),
                    copy_on_cross_device: true,
                }]);
            }
        }
        let mut candidates: Vec<Candidate> = Discovery::topdir_candidates_with_reporter(
            platform.fs(),
            platform.identity(),
            source_mount,
            true,
            reporter,
        )
        .into_iter()
        .map(|layout| Candidate {
            layout,
            copy_on_cross_device: false,
        })
        .collect();
        if let Some(home) = home {
            candidates.push(Candidate {
                layout: home,
                copy_on_cross_device: true,
            });
        }
        if candidates.is_empty() {
            return Err(Error::new(
                ErrorKind::NotFound,
                NO_TRASH_CANDIDATE_OPERATION,
                None,
            ));
        }
        Ok(candidates)
    }
}
