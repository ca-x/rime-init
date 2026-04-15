Read MILESTONE.md to understand current progress.

Then implement ALL items under "待补全" section:

1. **Download resume**: In api/mod.rs download_file(), support Range header for resume. If .tmp file exists, resume from offset.

2. **Cache reuse**: In SchemeUpdater/DictUpdater run(), check local cache SHA256 matches remote before downloading.

3. **Key file detection**: SchemeUpdater checks if rime_dir/lua/wanxiang.lua exists, force update if missing.

4. **Scheme switch detection**: Check if local_record.name matches current scheme_zip, re-download if mismatched.

5. **CNB tag fallback**: DictUpdater CNB query tries latest tag first, falls back to v1.0.0.

6. **Multi-engine sync**: Add sync_to_engines(src_dir, engines, exclude_files) to deployer/mod.rs.

7. **Update summary**: UpdateResult gets skipped_components vec and versions map. update_all reports skipped items.

8. **Unit tests**: Add #[cfg(test)] mod tests to fileutil/hash.rs, fileutil/extract.rs, updater/model_patch.rs, updater/mod.rs. Test hash, extract, patch/unpatch, needs_update.

9. **LICENSE**: Create MIT license file.

10. **Clean warnings**: Remove all unused imports and dead code. Must pass `cargo build` and `cargo test` with no errors.

Rules:
- Do NOT change file structure, only add to existing files
- cargo build must pass
- cargo test must pass
