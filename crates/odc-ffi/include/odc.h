/* odc.h - C ABI for the ODC core library. */
#ifndef ODC_H
#define ODC_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/*
 * Opaque handle to an opened ODC repository.
 *
 * Threading: a handle may be used by one thread at a time. Sharing a handle
 * between threads requires external synchronisation.
 */
typedef struct OdcRepo OdcRepo;

/*
 * Open an existing repository directory.
 *
 * On success returns a non-NULL handle and stores NULL in *error_message.
 * On failure returns NULL and stores a heap-allocated, NUL-terminated UTF-8
 * error message in *error_message; release it with odc_string_free().
 * error_message may be NULL if the caller does not want the diagnostic.
 */
OdcRepo *odc_repo_open(const char *path_utf8, char **error_message);

/*
 * Close a handle returned by odc_repo_open(). Passing NULL is a no-op.
 */
void odc_repo_close(OdcRepo *repo);

/*
 * Number of committed entries. Returns 0 on failure.
 */
int64_t odc_repo_entry_count(const OdcRepo *repo, char **error_message);

/*
 * Prefix-search normalised keys.
 *
 * limit must be between 1 and 200 inclusive. Returns a NUL-terminated JSON
 * array of {"key": "...", "display": "..."} objects in key order, or NULL on
 * failure. The returned string must be released with odc_string_free().
 */
char *odc_repo_search_json(const OdcRepo *repo, const char *query_utf8,
                           int32_t limit, char **error_message);

/*
 * Substring-search normalised keys and display spellings.
 *
 * limit must be between 1 and 200 inclusive. Returns a NUL-terminated JSON
 * array of {"key": "...", "display": "..."} objects, shortest key first, or
 * NULL on failure. The returned string must be released with
 * odc_string_free().
 */
char *odc_repo_search_substring_json(const OdcRepo *repo, const char *query_utf8,
                                     int32_t limit, char **error_message);

/*
 * Look up one entry by exact normalised key and return a NUL-terminated JSON
 * object {"key", "display", "phonetic", "defs", "raw_html", "extra"}.
 *
 * Returns NULL when the key is absent or on failure; *error_message then
 * carries the diagnostic. Release the result with odc_string_free().
 */
char *odc_repo_entry_json(const OdcRepo *repo, const char *key_utf8,
                          char **error_message);

/*
 * Resolve a word through `@@@LINK=` redirects and return a NUL-terminated JSON
 * object whose "status" is one of:
 *
 *   "found"     {"status","requested","key","hops","entry"}
 *   "missing"   {"status","requested","target","hops"}
 *   "cycle"     {"status","requested","chain"}
 *   "too_long"  {"status","requested","chain"}
 *   "not_found" {"status","requested"}
 *
 * "entry" carries the same object odc_repo_entry_json() returns. Returns NULL
 * on failure; *error_message then carries the diagnostic. Release the result
 * with odc_string_free().
 */
char *odc_repo_entry_link_json(const OdcRepo *repo, const char *key_utf8,
                               char **error_message);

/*
 * One page of working-set entries as a JSON array of entry objects (the
 * same shape odc_repo_entry_json returns).
 *
 * after_key_utf8 is NULL for the first page, otherwise the last key of the
 * previous page. limit must be between 1 and 500 inclusive. Returns NULL on
 * failure; *error_message then carries the diagnostic. Release the result
 * with odc_string_free().
 */
char *odc_repo_entries_page_json(const OdcRepo *repo, const char *after_key_utf8,
                                 int32_t limit, char **error_message);

/*
 * Insert or replace one entry in the working set.
 *
 * entry_json_utf8 accepts the object shape odc_repo_entry_json returns:
 * key, display, phonetic, defs, raw_html and extra. Returns a JSON object
 * {"key", "outcome"} where outcome is "added" or "modified". HEAD is not
 * touched until odc_repo_commit_json() succeeds.
 */
char *odc_repo_entry_put_json(OdcRepo *repo, const char *entry_json_utf8,
                              char **error_message);

/*
 * Remove one entry from the working set.
 *
 * Returns {"key", "outcome"} where outcome is "deleted" or "not_found".
 */
char *odc_repo_entry_delete_json(OdcRepo *repo, const char *key_utf8,
                                 char **error_message);

/*
 * Working-set status against HEAD.
 *
 * Returns {"added", "modified", "removed", "total", "entries", "blocks",
 * "head"}; head is null before the first commit.
 */
char *odc_repo_working_status_json(const OdcRepo *repo, char **error_message);

/*
 * Changed keys against HEAD as {"key", "kind"} objects in key order.
 *
 * kind is "added", "modified" or "deleted". after_key_utf8 is NULL for the
 * first page; limit must be between 1 and 500 inclusive.
 */
char *odc_repo_working_changes_json(const OdcRepo *repo, const char *after_key_utf8,
                                    int32_t limit, char **error_message);

/*
 * Field-level diff of one key between HEAD and the working set.
 *
 * Returns {"key", "kind", "fields": [{"field", "before", "after",
 * "changed"}]}. kind is "added", "modified", "deleted" or "absent";
 * before/after are null when the field does not exist on that side.
 */
char *odc_repo_entry_diff_json(const OdcRepo *repo, const char *key_utf8,
                               char **error_message);

/*
 * Commit the working set with the current wall-clock timestamp.
 *
 * Returns {"hash", "entry_count", "ts", "author", "message",
 * "new_blocks", "reused_blocks", "entries"}. An empty working set fails
 * with the usual ODC-E012 diagnostic.
 */
char *odc_repo_commit_json(OdcRepo *repo, const char *message_utf8,
                           const char *author_utf8, char **error_message);

/*
 * Discard every working-set change. HEAD is untouched.
 *
 * Returns {"discarded": true}.
 */
char *odc_repo_discard_json(OdcRepo *repo, char **error_message);




/*
 * Import an MDX file into a standalone .odc file.
 *
 * Returns 0 on success and -1 on failure. On success *report_json_out holds a
 * heap-allocated, NUL-terminated JSON report; release it with
 * odc_string_free(). On failure *report_json_out is NULL and *error_message
 * carries the diagnostic.
 *
 * The output is written atomically: the pack is staged next to the
 * destination and renamed into place. An existing file at out_odc_path is a
 * hard error and is never replaced.
 */
int32_t odc_import_mdx_to_odc(const char *mdx_path, const char *out_odc_path,
                              char **report_json_out, char **error_message);

/*
 * Number of MDD resources stored in the repository. Returns 0 on failure.
 */
int64_t odc_repo_resource_count(const OdcRepo *repo, char **error_message);

/*
 * Length in bytes of the resource stored under key_utf8.
 *
 * Returns 1 and writes the length through out_len when the resource exists,
 * 0 when it does not, and -1 on failure. The lookup is case-insensitive so
 * \pic\Apple.png and /pic/apple.png reach the same payload.
 */
int32_t odc_repo_resource_len(const OdcRepo *repo, const char *key_utf8,
                              uint64_t *out_len, char **error_message);

/*
 * Copy the resource stored under key_utf8 into buffer.
 *
 * Returns 1 when the payload was copied, 0 when it does not exist and -1 on
 * failure. capacity must be at least the length reported by
 * odc_repo_resource_len(); the copied length is written through out_len when
 * that pointer is not NULL.
 */
int32_t odc_repo_resource_copy(const OdcRepo *repo, const char *key_utf8,
                               uint8_t *buffer, size_t capacity,
                               size_t *out_len, char **error_message);

/*
 * Import an MDX file plus an explicit list of MDD volumes.
 *
 * mdd_paths_json is NULL to let the core discover the volumes next to the MDX
 * file, or a JSON array of absolute paths in import order. Returns 0 on
 * success (report_json_out holds the same report as odc_import_mdx_to_odc,
 * with mdd_files, resource_count, resource_bytes and warnings) and -1 on
 * failure.
 */
int32_t odc_import_mdx_to_odc_with_mdd(const char *mdx_path,
                                      const char *out_odc_path,
                                      const char *mdd_paths_json,
                                      char **report_json_out,
                                      char **error_message);

/*
 * Stage of an import, as passed to odc_import_progress_fn.
 *
 * ODC_IMPORT_STAGE_PREPARING: the MDD volumes are being opened to learn the
 *   resource total; done is 0 and total is that resource total.
 * ODC_IMPORT_STAGE_ENTRIES: dictionary records are being read; total is
 *   min(limit, records in the file).
 * ODC_IMPORT_STAGE_RESOURCES: MDD payloads are being copied; volume_index is
 *   one-based and volume_count is the number of readable volumes.
 * ODC_IMPORT_STAGE_COMMITTING: every record and resource is written, done and
 *   total are both the sum of the entry and resource totals.
 */
#define ODC_IMPORT_STAGE_PREPARING 0
#define ODC_IMPORT_STAGE_ENTRIES 1
#define ODC_IMPORT_STAGE_RESOURCES 2
#define ODC_IMPORT_STAGE_COMMITTING 3

/*
 * Progress callback for odc_import_mdx_to_odc_with_mdd_progress.
 *
 * Called on the importing thread: it must not block, do UI work or call back
 * into this library. context is the pointer the caller passed in, unchanged.
 */
typedef void (*odc_import_progress_fn)(int32_t stage, uint64_t done,
                                       uint64_t total, uint32_t volume_index,
                                       uint32_t volume_count, void *context);

/*
 * Import an MDX file plus MDD volumes and report progress while doing it.
 *
 * Same contract as odc_import_mdx_to_odc_with_mdd. callback may be NULL, in
 * which case this behaves exactly like the older entry point. The callback is
 * invoked at most about ten times per second, plus once on every stage change
 * and once for the final sample where done == total.
 */
int32_t odc_import_mdx_to_odc_with_mdd_progress(
    const char *mdx_path, const char *out_odc_path, const char *mdd_paths_json,
    odc_import_progress_fn callback, void *context, char **report_json_out,
    char **error_message);
/*
 * Import an MDX file into a new directory repository (an editable library).
 *
 * repo_dir must be absent or an empty directory; anything else is refused so
 * an existing library is never overwritten. The call imports entries and MDD
 * resources, then commits them, so HEAD exists when the function returns.
 * On failure a directory created by this call is removed again.
 *
 * The report carries the same fields as odc_import_mdx_to_odc plus
 * repo_path, commit and is_directory.
 */
int32_t odc_import_mdx_to_repo_with_mdd_progress(
    const char *mdx_path, const char *repo_dir, const char *mdd_paths_json,
    odc_import_progress_fn callback, void *context, char **report_json_out,
    char **error_message);

/*
 * Copy a single-file pack into a new directory repository without touching
 * the source file. repo_dir must be absent or empty.
 *
 * The report is {"repo_path", "commit", "entries", "is_directory"}.
 */
int32_t odc_copy_pack_to_repo(
    const char *pack_path, const char *repo_dir, char **report_json_out,
    char **error_message);


/*
 * Release a string returned by this library. Passing NULL is a no-op.
 */
void odc_string_free(char *s);

/*
 * Version of the FFI library, e.g. "0.1.0". The pointer is static storage
 * and must not be freed.
 */
const char *odc_version(void);

#ifdef __cplusplus
}
#endif

#endif /* ODC_H */
