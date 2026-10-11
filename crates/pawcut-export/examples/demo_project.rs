//! Monta um projeto de demonstração: vídeo em V1 cortado, logo em V2, título em V3 e música em A2.
//! Uso: cargo run -p pawcut-export --example demo_project -- video.mp4 logo.png musica.mp3 demo.pawcut

use pawcut_core::*;
use pawcut_media::probe;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [video, logo, music, out] = args.as_slice() else {
        eprintln!("uso: demo_project <video> <logo.png> <musica> <saida.pawcut>");
        std::process::exit(2);
    };
    let mut ed = Editor::new(Project::new(ProjectSettings::default()));
    let fps = ed.project().settings.fps;
    let add = |ed: &mut Editor, path: &str| -> Result<MediaInfo, Box<dyn std::error::Error>> {
        let info = probe(path.as_ref(), fps)?;
        ed.apply(Edit::AddMedia(info.clone()))?;
        Ok(info)
    };
    let v = add(&mut ed, video)?;
    let l = add(&mut ed, logo)?;
    let m = add(&mut ed, music)?;
    let id = |ed: &Editor, info: &MediaInfo| {
        ed.project()
            .media
            .iter()
            .find(|r| r.info == *info)
            .map(|r| r.id)
            .unwrap()
    };
    let (vid, lid, mid) = (id(&ed, &v), id(&ed, &l), id(&ed, &m));
    for _ in 0..2 {
        ed.apply(Edit::AddTrack(TrackKind::Video))?;
    }
    ed.apply(Edit::AddTrack(TrackKind::Audio))?;
    let p = ed.project().clone();
    let len = v.duration.unwrap_or(150).min(300);
    let draft = |source, duration| ClipDraft {
        source,
        source_start: 0.0,
        duration,
    };

    // Vídeo com o áudio dele, ligados.
    let mut items = vec![Placement {
        track: p.video_tracks[0].id,
        draft: draft(ClipSource::Media(vid), len),
    }];
    if matches!(v.kind, MediaKind::Video { has_audio: true }) {
        items.push(Placement {
            track: p.audio_tracks[0].id,
            draft: draft(ClipSource::Media(vid), len),
        });
    }
    ed.apply(Edit::Overwrite { at: 0, items })?;
    // Corta um pedaço do meio com ripple.
    ed.apply(Edit::Blade {
        at: len / 3,
        target: BladeTarget::AllTracks,
    })?;
    ed.apply(Edit::Blade {
        at: len / 2,
        target: BladeTarget::AllTracks,
    })?;
    if let Some(mid_clip) = ed.project().video_tracks[0].clip_at(len / 3) {
        let pair = ed.project().expand_linked(&[mid_clip.id]);
        ed.apply(Edit::RippleDelete { clips: pair })?;
    }
    let total = ed.project().duration();

    let logo_clip = ed.apply(Edit::Overwrite {
        at: 0,
        items: vec![Placement {
            track: p.video_tracks[1].id,
            draft: draft(ClipSource::Media(lid), total),
        }],
    })?;
    ed.apply(Edit::SetClip {
        clip: logo_clip.clips[0],
        change: ClipChange::Transform(Transform {
            x: 760.0,
            y: 400.0,
            scale: 0.5,
            opacity: 0.8,
            ..Default::default()
        }),
    })?;
    let title = TextStyle {
        text: "PawCut".into(),
        font: "Sans".into(),
        size: 96.0,
        color: Rgba::new(255, 255, 255, 255),
        ..Default::default()
    };
    let t = ed.apply(Edit::Overwrite {
        at: 0,
        items: vec![Placement {
            track: p.video_tracks[2].id,
            draft: draft(ClipSource::Text(title), 90.min(total)),
        }],
    })?;
    ed.apply(Edit::SetClip {
        clip: t.clips[0],
        change: ClipChange::FadeOut(20),
    })?;
    let music_len = m.duration.unwrap_or(total).min(total);
    let mc = ed.apply(Edit::Overwrite {
        at: 0,
        items: vec![Placement {
            track: p.audio_tracks[1].id,
            draft: draft(ClipSource::Media(mid), music_len),
        }],
    })?;
    ed.apply(Edit::SetClip {
        clip: mc.clips[0],
        change: ClipChange::VolumeDb(-12.0),
    })?;
    ed.apply(Edit::SetClip {
        clip: mc.clips[0],
        change: ClipChange::FadeOut(30),
    })?;

    save_project(ed.project(), out.as_ref())?;
    eprintln!("projeto salvo em {out} ({total} frames)");
    Ok(())
}
