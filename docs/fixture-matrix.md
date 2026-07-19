# Synthetic fixture capability matrix

Matrix version: 1

Every fixture is generated or hand-authored synthetic data. Names, paths, identifiers,
prompts, commands, and credentials are obvious canaries and do not derive from real Claude
history. The matrix is a Phase 0 plan; later phases add the fixture and the named assertion
before claiming the capability.

| ID | Phase | Synthetic capability | Fixture shape | Required assertions |
| --- | --- | --- | --- | --- |
| F001 | 1 | discovery precedence | transcript roots + env/default + repeatable OTLP files, duplicate paths/symlinks/hard links, public compatibility readers with `Some(year)`/`None`, legacy discovery helpers | per-kind command/default order, canonical/native-object dedup, safe aliases, exact-path privacy, explicit failures; fallible public readers preserve all-years selection; fallible discovery is bounded/root-confined and surfaces safe actionable errors |
| F002 | 1 | inaccessible/partial roots | unreadable file/root plus readable sibling | explicit failure vs implicit warning; indeterminate is not empty |
| F003 | 1 | transcript variants | assistant/user/system/progress/summary, main/sidechain/subagent | accepted capabilities and relationships match adapter version |
| F004 | 1 | malformed/unknown/boundary input | blank physical records, invalid JSON, oversized line, unknown kind/nested keys, token totals beyond `u64`, public compatibility entry point | bounded continuation, exact global/per-source counts and matching completeness, structural keys/types only, no values; aggregate counters saturate with a stable warning and partial capability instead of panic/wrap; public fallible reader returns the same bounded coverage |
| F005 | 1 | duplicate observations | repeated message/request IDs at equal and distinct instants, repeated content blocks, conflicting richness | exact-instant repeat produces one deterministic winner and duplicate count; distinct instants remain distinct |
| F006 | 1 | identity context | same message ID in main, sidechain, separate sessions, and byte-distinct non-UTF-8 project names | source-local/native-component key prevents false dedup without lossy path text |
| F007 | 1 | deterministic scheduling baseline | shuffled files/lines and two fresh privacy salts under the Phase 1 single-worker policy | byte-identical normalized snapshot/report and diagnostics ordering; salts never affect aliases/sorts/output |
| F008 | 1 | content denylist | prompts, response, command, tool args/results, `.env`-like secret, raw/JSON-escaped/percent/base64 IDs/paths/model/tool canaries through legacy public readers | standard events/store/logs and both compatibility projections contain no raw or encoded canary; aliases/classifications replace raw identity; redaction counts exact |
| F009 | 1 | OTLP events, producer contract, and resource limits | pinned file-exporter v0.148.0/pdata v1.54.0/slim-OTLP v1.10.0 metrics+logs; correct and conflicting metric wire units; alternate/incompatible exporter shape; oversized line/depth/resource/scope/point/attribute/cardinality cases | pinned shape/unit accepted with expected/unverified producer diagnostic; conflicting unit rolls back the object; incompatible shape bounded/unsupported; timestamps/capabilities; each configured limit rejects before unbounded work |
| F010 | 1 | OTel privacy matrix | prompt/response/tool/raw-body/email/account/session attributes | default reject/redact at adapter boundary; private rules explicit |
| F011 | 1 | OTel delta temporality | contiguous delta token/cost points | exact non-overlapping sum and period attribution |
| F012 | 1 | OTel cumulative temporality | stable start with monotonic points | differences, not repeated cumulative sum |
| F013 | 1 | OTel reset/gap/overlap | reset, unknown start, gap, two writers, overlap | no negative/double count; partial/unresolved diagnostics |
| F014 | 1 | OTel repeat import | identical export lines imported twice | idempotent rows/facts and duplicate import count |
| F015 | 1 | local boundary straddle | point window crosses day/year in chosen timezone | partial/indeterminate; no export-time assignment or prorating |
| F016 | 1 | cross-source overlap | correlated/unrelated transcript+OTel, repeated request groups, matching-limit degradation, and partial OTel-only request facts | maximum-cardinality/minimum-distance strong-ID replacement; never sum unresolved overlap; bounded groups; sole partial OTel remains canonical with limitation |
| F017 | 1 | source removal | file deletion/truncation/replacement/rename after successful scan | stale rows removed, parent/subagent relations reconciled |
| F018 | 1/5 | failed reconciliation | root or one nested subtree becomes temporarily inaccessible | last-known rows preserved, partial/indeterminate coverage surfaced, and no deletion published until an authoritative successful scan |
| F019 | 2 | timezone boundaries | UTC/local year crossover, two offsets, deterministic host/default resolution | every calendar metric uses selected IANA timezone; valid host zone is selected and failed host resolution emits the UTC warning |
| F020 | 2 | DST, skipped dates, and leap day | gap/fold instants, an entirely skipped local date, and Feb 29 | deterministic instant attribution; no synthetic wall times or zero-length day buckets |
| F021 | 2 | resumed sessions | two bursts separated by days | elapsed span is long; active-time cap excludes idle gap |
| F022 | 2 | active interval union | overlapping main/subagent/tool intervals and a real parent/subagent group across midnight | union prevents double count; clipping/splitting exact; project/model/session inclusive projections and parent-group ownership are explicitly non-additive |
| F023 | 2 | zero/one/out-of-order/boundary sessions | empty, singleton, reversed timestamps, one session across adjacent windows | zero elapsed/active; stable ordering; distinct once per query/day and non-additive wider count |
| F024 | 2 | token categories | all six categories with exact zero, missing, partial, TTL composition, and saturation | categories remain separate; absent differs from zero; value/availability/samples/overflow/method/limitations agree from JSON through every renderer |
| F025 | 2 | cache denominators | positive, explicit-zero, missing/partial, and saturated denominators | unavailable on zero/incomplete/saturated evidence; exact documented ratios otherwise; every renderer agrees |
| F026 | 2 | model mapping | exact current IDs, provider prefixes, unknown/future IDs, direct fast modifier | exact mapping; unknown and unsupported modifiers remain unpriced |
| F027 | 2 | price coverage | source estimate, local price, unknown model, modifier/TTL cases | distinct values/provenance; priced share; no tier fallback |
| F028 | 2 | price effective dates | model whose price changes inside supported registry plus pre-launch and post-retirement observations | timestamp selects only a valid availability/price interval and report pins registry |
| F029 | 2 | deterministic report | fixed source bytes/options run twice | byte-identical canonical JSON; stable generated/data-through semantics |
| F030 | 2 | renderer agreement | rich plus unavailable/partial/saturated canonical reports | every common fact ID/value/period/method/sample/overflow/limitation agrees across outputs |
| F031 | 3 | adjacent comparison | current/prior equal windows, positive/zero baseline, direct all-zero activity, and clean aggregate zero intervals that omit each day's opening instant | absolute/relative delta rules, active dates, compatible signatures, and no prior-window waiver without an exhaustive producer-coverage declaration |
| F032 | 3 | partial comparison | one window incomplete, source policy changes, insufficient dates, and a mathematically valid comparison window outside the selected period | comparison unavailable with the exact coverage/period limitation |
| F033 | 3 | robust trend | flat/rising/falling series, one extreme point, and multiple observations per active date | declared trend method, daily-point sample semantics, complete proof fields, and minimum sample behavior |
| F034 | 3 | active efficiency | tokens/requests/errors over unioned active time | exact rate, threshold disclosure, no productivity wording |
| F035 | 3 | reliability | OTel requests, exhausted errors, retries, missing telemetry | exact definition/rate; missing is unavailable, not zero |
| F036 | 3 | tool behavior | transcript tool occurrence plus OTel result/status/latency/edit decisions, result-only evidence, and a finite extreme duration beyond 24 hours | capability-specific metrics and minimum samples; a direct result does not fabricate transcript occurrence; out-of-range latency is unavailable and no non-finite fact is emitted |
| F037 | 3 | routing | at least 11 mapped models plus unknown observations with request/token/cost shares, and exact model-only requests without token attributes | full-population denominators, deterministic `other-mapped` tail, request mapping independent of token/pricing evidence, shares close to 100%, unknown coverage, and no intent/quality inference |
| F038 | 3 | concentration | one/many projects with known weights | HHI/top share/project count; aliases only in standard output |
| F039 | 3 | anomalies | median/MAD normal, outlier, MAD zero, small sample | robust score and guardrails; descriptive language only |
| F040 | 3 | recommendation proof | each rule at below/at/above threshold plus an eligible tool at display rank 11 | evaluate the full analytical candidate population, emit only with required facts, retain a factual trigger-card reference within the ten-tool-card budget, and attach the bounded experiment |
| F041 | 3 | forbidden narratives | proxy patterns resembling throttle/cache reset/savings across all source templates, every Markdown contract including the root README, rendered text, and every manifest-pinned screenshot | unsupported causal or fixed-savings text never emitted; a synthetic positive README claim proves the documentation guard runs; OCR canaries prove each screenshot was read |
| F042 | 3 | partial telemetry | transcript-only, metrics-only, events-only, OTel-only partial, strongly correlated transcript/OTel overlap, and exact transcript usage beside unrelated malformed tool telemetry | sole-source observed totals retained; overlap selected once; unrelated degradation leaves trend/anomaly available; capability matrix/coverage and graceful insight suppression |
| F043 | 4 | standard privacy | all hostile canaries in every sensitive source field | terminal/JSON/HTML/Markdown/card/store/log scan stays clean |
| F044 | 4 | encoded privacy | HTML entities, JSON escapes, Unicode, path/name substrings | decoded and raw scans find no sensitive canary |
| F045 | 4 | private profiles | explicit private and private-content invocations | exact fields only in authorized profile; warnings to stderr |
| F046 | 4 | share-card type barrier | attempted construction with identifiers/content | compile-time/serialization surface cannot carry private fields |
| F047 | 4 | hostile rendering | `<script>`, ANSI/control chars, long Unicode, bidi controls | safe escaping/sanitization and bounded layout |
| F048 | 4 | CLI JSON discipline | success, configuration/ingestion/empty errors, and every file/open conflict plus `--json` | stdout is one JSON value; ordinary stderr is empty; conflicts exit 2 with no side effect; explicit private diagnostics remain labeled stderr |
| F049 | 4 | empty/partial UX | empty, retention-limited, malformed-rich snapshot | `E_NO_RECORDS` plus clear next action; partial/indeterminate openings use observed-activity language |
| F050 | 4 | archive and output isolation | explicit archive with content canaries, simultaneous standard exports, prior standard-file sentinels, pre-existing root/file symlinks, permissive umask, denied hard-link syscalls, injected mid-write failure, simulated crash after destination renames, competing archive/final destination, a standard destination replaced after installation, and Windows ACL-capability check | archive alone contains content; existing/competing roots are refused without target mutation; new directories/files are owner-only; hard links are not required; unambiguous output failure and next-invocation crash recovery restore every prior standard file and remove staging; ambiguous rollback preserves the competitor and displaced report, retains the prior file in named owner-only recovery staging, and returns an error; Windows volumes without enforceable ACLs fail before content; all other artifacts remain clean |
| F051 | 5 | large transcript corpus | deterministic generator with millions of mixed records | correct totals vs small oracle, bounded memory, no malformed loss |
| F052 | 5 | large OTel corpus | paired cross-file streams with delta and cumulative points, reset, gap, overlap, request overlap, and sensitive canaries | exact accepted/filtered metric-point oracle, canonical token delta, stable reset/gap/overlap diagnostics, and bounded cardinality/memory |
| F053 | 5 | cold/warm/no-store | same corpus through each mode | report equality and measured latency/IO/RSS |
| F054 | 5 | incremental append | append small tail to large existing corpus | only changed source work; exact result equality |
| F055 | 5 | incremental mutation | deletion/truncate/replace/rename, root failure, nested-subtree failure, and a readable-file mutation during a different inaccessible subtree | reconciliation rules, retained-row safety, readable-inventory rejection, and transactional recovery |
| F056 | 5 | corrupt store/migration | genuine legacy table layouts for formats 1–8, a source failure after staged migration preparation, corrupt bytes, and a failed complete rebuild scan | every old layout rebuilds derived state; the interrupted migration leaves the legacy database byte-for-byte unchanged before a successful retry; a failed rebuild preserves the prior database and leaves no staging residue; corruption is actionable; no source mutation/data leak |
| F057 | 5 | parallel determinism | hardware-aware worker counts and randomized delays | source-current identical output across valid worker counts; historical throughput/duration/utilization remain informational telemetry |
| F058 | 6 | public/JSON compatibility | Phase 0 API artifact + schema v1/v2 goldens | intended changes classified, shims/deprecations/migration verified |
| F059 | 6 | documentation commands | README/help examples against built binary | every command/flag/output claim executes as documented |
| F060 | 6 | final privacy corpus | aggregate of every hostile fixture | repository artifacts and generated standard outputs stay clean |

## Phase 1 executable evidence

The Phase 1 rows are implemented in `tests/phase1_ingestion.rs` plus bounded unit tests
inside the private adapters. This table names the regression entry points so the matrix is
auditable rather than aspirational.

| ID | Executable evidence |
| --- | --- |
| F001 | `repeatable_explicit_roots_feed_one_report_in_command_order`, `claude_config_dir_precedes_the_supported_home_default`, `canonical_duplicate_roots_import_once_and_keep_command_order_aliases`, `hard_linked_transcript_is_scanned_once`, `transcript_directory_entries_share_one_invocation_budget`, `exact_source_paths_require_explicit_private_diagnostics`, `json_ingestion_failures_have_a_stable_safe_actionable_shape`, `public_compatibility_readers_preserve_all_years_semantics`, `public_fallible_readers_surface_bounds_coverage_and_safe_errors`, `legacy_discovery_rejects_out_of_root_symlinks`, `compatibility_discovery_is_bounded_fallible_and_scope_accurate`, `compatibility_discovery_surfaces_the_depth_limit` |
| F002 | `missing_implicit_transcripts_are_visible_when_explicit_otel_is_usable`, `unreadable_implicit_transcripts_are_visible_when_explicit_otel_is_usable`, `unreadable_implicit_transcript_is_fatal_without_a_usable_source`, `unreadable_explicit_file_is_an_actionable_error_not_empty_history`, `transcript_symlink_escape_is_excluded_and_marks_coverage_partial`, `empty_explicit_history_returns_machine_readable_empty_coverage` |
| F003 | `known_transcript_variants_and_sidechain_context_share_the_normalized_stream` |
| F004 | `malformed_unknown_and_duplicate_records_are_separately_counted`, `rejected_transcript_record_does_not_consume_public_aliases`, `degradation_unknown_records_downgrade_analytical_claims`, `partial_cost_coverage_separates_local_and_source_cost_sums`, `aggregate_token_totals_saturate_at_u64_boundary`, `provably_out_of_period_records_do_not_downgrade_analytical_claims`, `oversized_and_deep_transcript_lines_are_bounded_and_later_records_survive`, `out_of_order_transcript_records_produce_stable_chronological_facts`, `public_fallible_readers_surface_bounds_coverage_and_safe_errors`, `timestamp_date_key_validates_and_normalizes_to_utc`, `public_daily_aggregation_uses_validated_utc_dates`, `hostile_multibyte_public_timestamps_do_not_panic`, `html_project_bars_handle_maximum_public_token_values` |
| F005 | `richer_duplicate_wins_and_decision_is_salt_independent`, `equal_richness_duplicate_order_is_deterministic`, `strong_message_identity_collapses_repeated_transcript_roots`, `repeated_request_at_the_same_source_timestamp_remains_a_duplicate`, `repeated_request_ids_use_maximum_cross_source_matching` |
| F006 | `duplicate_identity_is_scoped_by_session_context`, `known_transcript_variants_and_sidechain_context_share_the_normalized_stream`, `repeated_request_ids_in_distinct_otel_sessions_remain_distinct`, `non_utf8_project_names_remain_distinct` |
| F007 | `ingestion_execution_policy_is_single_worker`, `rejected_transcript_record_does_not_consume_public_aliases`, `richer_duplicate_wins_and_decision_is_salt_independent`, `equal_richness_duplicate_order_is_deterministic`, `out_of_order_transcript_records_produce_stable_chronological_facts`, `aggregate_request_matching_budget_degrades_independently_of_record_order`, `report_is_invariant_to_ambient_tz`, `resolve_project_path_breaks_equal_count_ties_lexically`, `session_top_tools_break_equal_count_ties_lexically`, `story_top_tool_uses_the_same_lexical_tie_policy_as_session_intelligence`, `busiest_hour_breaks_equal_count_ties_by_earliest_hour`; this is the Phase 1 single-worker and fresh-salt baseline, while F057 exclusively owns the later 1-versus-N scheduling proof |
| F008 | `standard_report_excludes_content_paths_and_raw_identifiers`, `arbitrary_model_and_tool_names_are_classified_before_standard_storage`, `every_standard_renderer_excludes_sensitive_canaries`, `terminal_hostile_public_report_strings_are_inert`, `public_compatibility_readers_use_the_privacy_safe_normalized_projection`, `public_compatibility_reader_sources_have_no_independent_raw_adapter`, `archive_is_the_only_explicit_content_bearing_sidecar`, `private_archive_entrypoints_are_bounded`, `archive_renders_hostile_prompts_as_inert_markdown`, `hostile_report_values_remain_literal`, `output_transaction_refuses_standard_symlinks`, `output_rollback_preserves_competing_standard_destination`, `browser_launch_failure_is_actionable_after_outputs_commit`, `linux_binary_does_not_import_glibc_renameat2_wrapper` |
| F009 | `pinned_otel_api_request_is_accepted_without_sensitive_attributes`, `otel_pinned_integer_wire_encoding`, `otel_rejects_conflicting_metric_units`, `every_supported_otel_log_event_surfaces_its_direct_capability`, `every_supported_otel_metric_retains_a_named_capability`, `otel_invalid_optional_usage_does_not_become_zero_or_grade`, `otel_explicit_zero_usage_remains_observed`, `otel_token_metric_categories_are_collectively_available`, `otel_source_cost_and_token_metrics_do_not_form_one_cost_claim`, `disjoint_otel_requests_do_not_suppress_aggregate_metrics`, `overlapping_and_ambiguous_otel_metrics_are_not_summed_with_requests`, `degradation_unknown_records_downgrade_analytical_claims`, `provably_out_of_period_records_do_not_downgrade_analytical_claims`, `source_capabilities_are_linear_in_events_and_sources`, `otel_inherited_attribute_merge_work_is_linear`, `incompatible_otel_shape_is_counted_without_guessing_partial_facts`, `otel_attribute_limit_rejects_the_export_object_before_partial_acceptance`, `otel_scope_limit_is_enforced_before_resource_filtering`, `otel_resource_record_point_and_text_limits_reject_whole_objects`, `oversized_and_deep_otel_lines_are_bounded_and_later_exports_survive`, `metric_stream_cardinality_rejects_only_new_identities_at_the_limit`, `otel_many_source_checkpointing_is_bounded` |
| F010 | `pinned_otel_api_request_is_accepted_without_sensitive_attributes`, `content_bearing_otel_events_are_excluded_without_copying_bodies`, `every_standard_renderer_excludes_sensitive_canaries` |
| F011 | `delta_gaps_are_partial_and_distinct_writer_keys_do_not_interfere`, `overlapping_delta_is_excluded_but_exact_repeat_is_idempotent` |
| F012 | `cumulative_otel_metrics_use_differences_and_repeat_import_is_idempotent`, `cumulative_metric_state_continues_across_selected_files`, `cumulative_metric_state_is_selected_file_order_independent`, `cumulative_metric_identity_is_attribute_order_independent` |
| F013 | `cumulative_points_emit_differences_and_resets_without_negative_values`, `changed_cumulative_start_cannot_hide_an_overlap`, `delta_gaps_are_partial_and_distinct_writer_keys_do_not_interfere`, `overlapping_delta_points_reconcile_physical_records` |
| F014 | `cumulative_otel_metrics_use_differences_and_repeat_import_is_idempotent`, `strong_request_identity_collapses_repeated_otel_files` |
| F015 | `metric_boundary_straddle_is_partial_and_never_prorated`, `metric_same_year_day_boundary_straddle_is_filtered`, `metric_midnight_endpoint_obeys_the_half_open_day_contract`, `provably_out_of_period_records_do_not_downgrade_analytical_claims` |
| F016 | `strong_request_identity_prefers_otel_without_double_counting`, `subagent_request_identity_correlates_across_sources`, `repeated_request_ids_use_maximum_cross_source_matching`, `maximum_cross_source_matching_minimizes_total_timestamp_distance`, `equal_distance_request_matching_is_record_order_independent`, `fallback_cross_source_identity_preserves_distinct_facts`, `oversized_request_correlation_groups_degrade_with_a_bounded_warning`, `aggregate_request_matching_budget_degrades_independently_of_record_order`, `request_identity_never_correlates_across_sessions_or_incompatible_times`, `unresolved_cross_source_overlap_keeps_transcript_authority`, `disjoint_transcript_and_otel_metric_are_both_canonical`, `overlapping_transcript_and_otel_metric_remains_unresolved`, `correlated_otel_request_supersedes_metric_with_transcript_present`, `disjoint_otel_requests_do_not_suppress_aggregate_metrics`, `overlapping_and_ambiguous_otel_metrics_are_not_summed_with_requests`, `non_usage_metrics_do_not_invent_assistant_messages`, `otel_invalid_optional_usage_does_not_become_zero_or_grade` |
| F017 | `deleted_source_file_cannot_survive_a_fresh_no_store_scan`, `truncation_replacement_and_rename_are_reconciled_by_each_fresh_scan`, `transcript_discovery_open_replacement_is_rejected`, `transcript_nested_directory_mutation_is_fatal`, `transcript_directory_mutation_after_discovery_is_fatal`, `otel_discovery_open_replacement_is_rejected` |
| F018 | `unreadable_explicit_file_is_an_actionable_error_not_empty_history`, `f055_store_reconciles_mutations_and_preserves_rows_on_root_failure`, `f055_store_retains_last_known_report_when_one_transcript_subtree_is_inaccessible`, `f055_partial_subtree_rejects_stale_readable_inventory`; no-store Phase 1 has no cached rows to preserve, while the selected store proves root-failure non-mutation, nested-subtree retained partial coverage, and fail-closed readable-file validation |

## Phase 2 executable evidence

Phase 2 is implemented in `tests/phase2_metrics.rs` plus the production reconciliation
unit gate in `src/ingestion/mod.rs`. Every test uses an explicit synthetic source and
isolated home/config/TZ.

| ID | Executable evidence |
| --- | --- |
| F019 | `f019_selected_iana_timezone_controls_year_day_hour_and_labels`, `default_timezone_uses_valid_host_zone_and_warns_on_utc_fallback`, `observed_day_span_counts_inclusive_local_calendar_dates`, `invalid_timezone_is_actionable_and_json_safe` |
| F020 | `f020_dst_gap_fold_and_leap_day_attribute_real_instants_only`, `f020_skipped_local_date_uses_next_real_instant`, `f022_period_clipping_local_midnight_and_dst_use_real_instant_durations` |
| F021 | `f021_resumed_session_keeps_elapsed_and_capped_active_time_separate` |
| F022 | `f022_overlapping_main_subagent_and_direct_intervals_union_once`, `f022_parent_group_and_dimension_inclusive_values_are_non_additive`, `f022_period_clipping_local_midnight_and_dst_use_real_instant_durations` |
| F023 | `f023_singleton_out_of_order_and_threshold_boundaries_are_deterministic`, `f022_period_clipping_local_midnight_and_dst_use_real_instant_durations`, `nonnumeric_active_threshold_is_actionable_and_json_safe`, Phase 1 `empty_explicit_history_returns_machine_readable_empty_coverage` |
| F024 | `f024_explicit_zero_remains_distinct_from_an_absent_token_category`, `f024_aggregate_metric_categories_form_one_complete_observation`, `f024_ttl_composition_overflow_and_dimensions_reconcile`, `f030_every_renderer_exposes_the_same_canonical_fact_lines_without_causal_cache_text`, `production_reconciliation_gate_rejects_a_perturbed_token_projection`, `production_reconciliation_gate_rejects_a_perturbed_activity_projection`, `production_reconciliation_gate_rejects_a_perturbed_cost_projection` |
| F025 | `f025_cache_shares_use_documented_denominators_and_zero_is_unavailable`, analyzer unit tests `analyze_cache_health_retains_totals_but_neutralizes_unsupported_derivations`, `analyze_cache_health_never_turns_a_zero_or_low_share_into_a_grade`, and `detect_inflection_points_is_a_neutral_compatibility_adapter`, `f030_every_renderer_exposes_the_same_canonical_fact_lines_without_causal_cache_text` (the integration test renders positive, zero, partial, and saturated cases through every output) |
| F026 | `f026_exact_first_party_provider_prefixes_map_without_tier_guessing`, `f026_fast_pricing_modifier_stays_unpriced_without_an_exact_registry_record`, `f026_f027_f028_exact_model_prices_effective_dates_and_unknown_coverage`, `f026_model_availability_boundaries_and_suspensions_remain_unpriced`, unit gate `embedded_registry_matches_the_dated_row_level_evidence_inventory`, `f028_legacy_cost_and_routing_use_only_the_local_api_equivalent_domain` |
| F027 | `f027_cache_ttl_prices_once_and_partner_provider_stays_unpriced`, `f027_cache_ttl_components_exceeding_generic_total_stay_unpriced`, `f027_cache_ttl_component_sum_overflow_stays_unpriced`, `f027_aggregate_metric_without_session_reconciles_as_unattributed_cost`, `f026_f027_f028_exact_model_prices_effective_dates_and_unknown_coverage`, `f024_ttl_composition_overflow_and_dimensions_reconcile`, `production_reconciliation_gate_rejects_a_perturbed_cost_projection`, `production_reconciliation_gate_rejects_cost_token_domain_drift` |
| F028 | `f026_f027_f028_exact_model_prices_effective_dates_and_unknown_coverage`, `f026_model_availability_boundaries_and_suspensions_remain_unpriced`, unit gate `embedded_registry_matches_the_dated_row_level_evidence_inventory`, `f028_legacy_cost_and_routing_use_only_the_local_api_equivalent_domain` |
| F029 | `f029_explicit_zone_json_is_byte_deterministic_across_runs_and_ambient_tz` |
| F030 | `f030_every_renderer_exposes_the_same_canonical_fact_lines_without_causal_cache_text` |

## Phase 3 executable evidence

Phase 3 is implemented in `tests/phase3_insights.rs` plus the production insight
reconciliation unit gate in `src/ingestion/insights.rs`. Every command uses synthetic
transcript or pinned local OTel fixtures with isolated home/config/TZ.

| ID | Executable evidence |
| --- | --- |
| F031 | `f031_adjacent_comparison_has_exact_windows_delta_and_zero_baseline_semantics`, `f031_incomplete_metric_day_intervals_cannot_waive_the_prior_active_day_gate`, `production_insight_reconciliation_rejects_a_fabricated_zero_baseline_waiver` |
| F032 | `f032_partial_or_incompatible_windows_suppress_comparison` |
| F033 | `f033_median_halves_trend_resists_one_extreme_point`, `f033_trend_point_samples_are_daily_points`, `f033_trend_boundaries_cover_flat_falling_zero_minimum_and_recent_cap`, `production_insight_reconciliation_rejects_trend_method_mutations` |
| F034 | `f034_active_efficiency_uses_unioned_active_seconds_and_observed_language` |
| F035 | `f035_reliability_uses_terminal_outcomes_and_recovered_retry_evidence` |
| F036 | `f036_tool_behavior_separates_results_latency_and_edit_decisions`, `f036_direct_result_does_not_create_occurrence` |
| F037 | `f037_routing_reports_mapped_and_unknown_shares_without_quality_inference`, `f037_model_mapping_without_token_evidence` |
| F038 | `f038_project_concentration_uses_known_output_hhi_and_safe_aliases`, `project_concentration_keeps_unattributed_weight_outside_hhi_and_aliases_out_of_share` |
| F039 | `f039_anomalies_use_median_mad_and_the_mad_zero_guard` |
| F040 | `f040_recommendation_rules_obey_below_at_and_above_thresholds` |
| F041 | `f041_insight_narratives_exclude_unsupported_causes_and_fixed_savings` |
| F042 | `f042_partial_telemetry_preserves_supported_facts_and_family_absence`, `dropped_attributes_only_weaken_their_event_family` (transcript/metrics/events, strong mixed-source overlap, unrelated unsupported-event preservation, unrelated malformed-tool isolation, malformed direct-denominator suppression, resource/scope/record dropped-attribute inheritance and reciprocal event-family isolation) |

`entertainment_labels_are_sample_gated_marked_and_deterministic` proves the entertainment
boundary and renderer marker. `insight_facts_reconcile_across_terminal_html_markdown_and_share_card`
proves the shared projection and share alias exclusion.
`terminal_width_caps_hostile_environment_values` drives the production width-normalization
helper with a billion-column ambient value and proves terminal rules, padding, charts, and bars
remain within the documented 40–512-column allocation bound.
`production_insight_reconciliation_rejects_fact_method_sample_and_window_mutations`,
`production_insight_reconciliation_rejects_noncomparison_arithmetic_mutations`, and
`production_insight_reconciliation_rejects_arithmetic_mutations_across_every_family`,
plus the trend-specific mutation gate,
mutate each proof dimension plus every family arithmetic/reference path and require
`E_INSIGHT_RECONCILIATION`.

## Phase 4 executable evidence

`tests/phase4_product.rs` owns the profile, hostile-rendering, trust, CLI, and output
selection matrix; existing Phase 1 transaction tests remain the syscall/ACL/race authority
for F050.

| ID | Executable evidence |
| --- | --- |
| F043 | `f043_standard_and_share_surfaces_exclude_sensitive_field_canaries`, `every_standard_renderer_excludes_sensitive_canaries` |
| F044 | `f044_encoded_sensitive_values_remain_absent_after_raw_and_decoded_scans` |
| F045 | `f045_private_profiles_require_explicit_opt_in_and_stay_isolated`, `archive_is_the_only_explicit_content_bearing_sidecar` |
| F046 | `f046_share_projection_excludes_private_carriers`, `typed_share_projection_has_no_private_field_or_value_carrier` |
| F047 | `f047_html_and_terminal_neutralize_controls_and_bidi`, `terminal_writer_failures_propagate`, `hostile_report_values_remain_literal`, `terminal_hostile_public_report_strings_are_inert`, `c4_terminal_widgets_use_unicode_display_columns`, `public_widgets_bound_hostile_widths_without_changing_ordinary_widths` |
| F048 | `f048_json_success_and_failures_preserve_single_value_stdout_discipline`, `f048_json_conflicts_with_file_and_open_flags`, `c4_json_conflicts_with_file_and_open_flags` |
| F049 | `f049_partial_histories_open_as_observed_activity`, `exported_renderers_surface_partial_and_indeterminate_coverage` |
| F050 | `f050_default_json_and_private_content_outputs_remain_isolated`, binary `archive_tests`, Phase 1 symlink/no-clobber/rollback/permission/syscall tests |

`c4_open_standalone_implies_standard_html_and_waits_for_success`,
`c4_open_selection_matrix_adds_html_only_when_needed`, and
`c4_nonzero_browser_launcher_status_returns_e_browser_open` prove the browser-selection
and child-status contract after transaction commit.

## Phase 5 executable evidence

`tests/phase5_scale.rs` owns the generated-corpus oracle, store, incremental, recovery, and
parallel-determinism matrix. `scripts/phase5-benchmark.sh` owns the repeated raw timing,
RSS, I/O, scaling, and utilization evidence recorded in
[`benchmarks/phase5-record.json`](benchmarks/phase5-record.json).

| ID | Executable evidence |
| --- | --- |
| F051 | `f051_generator_is_byte_deterministic_and_manifest_bounded`, `f051_oracle_matches_real_ingestion_and_excludes_canaries` |
| F052 | `f052_generated_otel_metrics_cover_temporality_reset_gap_and_overlap`, with F051's end-to-end token/coverage oracle (the generated OTLP/JSON contains cross-file delta and cumulative streams and exact reset/gap/overlap dispositions) |
| F053 | `f053_cold_first_warm_and_no_store_json_are_byte_identical`, `f053_cached_report_rejects_recomputed_private_output_carriers`; historical benchmark `first-import`, `warm-store`, and `startup` series are informational only |
| F054 | `f054_incremental_append_reads_only_changed_and_new_source_files`, benchmark `incremental` series |
| F055 | `f055_store_reconciles_mutations_and_preserves_rows_on_root_failure`, `f055_store_retains_last_known_report_when_one_transcript_subtree_is_inaccessible`, `f055_partial_subtree_rejects_stale_readable_inventory`, `f055_inaccessible_root_cannot_reuse_a_different_sources_retained_report` |
| F056 | `f056_store_is_private_corruption_is_explicit_and_rebuild_is_source_safe` (distinct legacy schemas 1–8 plus failed staged-migration preservation/retry); native Windows CI also executes this test against the protected current-user DACL implementation |
| F057 | `f057_parallel_standard_workers_preserve_json_serial_private_policy_is_invariant_and_panics_fail_closed`, benchmark `determinism`, `scale`, `memory`, and continuous `saturate` series |

## Fixture generator rules

The large-corpus generator fixes the seed, source/file/record distributions, duplicate and
malformed rates, OTel stream topology, and timestamp window in its versioned corpus classes;
only the bounded saturation target size is adjustable. It emits a machine-computable
coverage, metric, active-time, insight-eligibility, and token oracle alongside source files.
Generation time is excluded from the ingestion benchmark.

Every fixture declares capabilities and expected diagnostics. Golden files pin schema and
presentation only after semantic assertions pass. Tests use temporary homes/config roots
and never discover the developer's real Claude directory.
