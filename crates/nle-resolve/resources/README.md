# DaVinci Resolve 21 Native Template

`resolve21_native_template.drp` is a clean, media-free DaVinci Resolve 21 project archive scaffold derived from [`Ersiter/pr2resolve`](https://github.com/Ersiter/pr2resolve) (MIT License; see root `THIRD_PARTY_NOTICES.md`).

It supplies Resolve's baseline internal schema containers (`project.xml`, `MediaPool/Master/MpFolder.xml`, `SeqContainer/*.xml`, `Gallery.xml`, and verified Rec.709 / ACEScct / Rec.2100 HLG `SetupBA` blobs) which `nle-resolve` mutates and populates dynamically when generating native `.drp` archives.
