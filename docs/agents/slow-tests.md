# Slow tests: nightly, not per pull request

Issue #151 (build-speed fix 3). The `test` job runs `tools/gates.sh`, which runs the Rust tests with
`cargo nextest run --profile pr`. That profile leaves out the 26 slowest tests (20 s or more each in
a debug build, about 920 s of test time together). `.github/workflows/nightly.yml` runs **every** test with
`--profile nightly`, daily and on demand (`workflow_dispatch`). The list lives in `.config/nextest.toml`.

Nothing is deleted. A test that is renamed drops out of the list and so runs in the pull-request job again.
To move a test, edit the filter in `.config/nextest.toml` and the table below. To run all tests locally:
`GATES_NEXTEST_PROFILE=nightly bash tools/gates.sh`.

| Test (binary: name) | Debug time | Why it moved |
|---|---|---|
| owd-cli::volumes: `the_chain_gives_the_same_document_as_run_on_the_same_inputs_and_settings` | 101 s | Runs the whole chain twice through the CLI on the full river and DEM. |
| owd-format::optimise: `volumes_measure_the_lattice_option_like_any_other_under_the_pool_levels` | 63 s | Optimises a lattice option, then measures volumes at every pool level. |
| owd-cli::volumes: `export_writes_the_quantity_table_and_the_options_with_the_screening_label` | 54 s | Full alignment, optimise and volumes run before the export. |
| owd-cli::volumes: `optimise_adds_the_lattice_option_and_volumes_measure_it` | 53 s | Same full pipeline with the lattice option. |
| owd-core: `alignment::rolling_circle::water::tests::at_46_m_the_fairway_fit_places_every_bend` | 50 s | Fairway fit on every bend of the synthetic river. |
| owd-format::optimise: `weirs_pools_and_the_lattice_option_equal_the_core_called_directly` | 50 s | Runs the optimiser twice (stage and core) to compare. |
| owd-core: `alignment::fairway_fit::corridor_tests::a_narrow_creek_is_never_fitted_sharper_than_its_input` | 44 s | Long corridor sweep over a narrow creek. |
| owd-core: `alignment::rolling_circle::water::tests::at_40_m_every_apex_is_too_narrow_and_the_option_keeps_a_line` | 42 s | Fairway fit sweep at 40 m width. |
| owd-format::alignment: `a_stage_run_runs_the_rolling_circle_and_a_new_vessel_makes_it_stale_then_run_reruns_both` | 37 s | Runs the rolling circle twice end to end. |
| owd-cli::volumes: `run_reruns_a_stale_volumes_stage` | 36 s | Full run, edit, then full re-run through the CLI. |
| owd-format::volumes: `run_stale_reruns_volumes_when_alignment_changes` | 34 s | Full volumes run, edit, then re-run. |
| owd-cli::volumes: `a_changed_dem_warns_on_open_and_marks_volumes_stale` | 30 s | Full volumes run before the staleness check. |

| owd-format::volumes: `the_volumes_equal_section_volumes_called_directly` | 29 s | Long run on the full river or DEM in a debug build. |
| owd-cli::volumes: `volumes_equal_the_core_called_directly_on_the_same_dem` | 27 s | Long run on the full river or DEM in a debug build. |
| owd-core: `alignment::fairway_fit::property_tests::the_synthetic_river_fits_at_300_m_and_is_too_narrow_at_every_apex_at_40_m` | 25 s | Long run on the full river or DEM in a debug build. |
| owd-format::volumes: `a_new_dem_reference_marks_volumes_stale` | 24 s | Long run on the full river or DEM in a debug build. |
| owd-format::volumes: `a_saved_document_reloads_with_the_same_volumes` | 24 s | Long run on the full river or DEM in a debug build. |
| owd-format::volumes: `a_changed_dem_file_warns_on_open_and_marks_volumes_stale` | 23 s | Long run on the full river or DEM in a debug build. |
| owd-cli::cdesign: `align_adds_a_rolling_circle_option_and_replaces_it_when_run_again` | 23 s | Long run on the full river or DEM in a debug build. |
| owd-format::wgs84_river: `a_wgs84_river_is_reprojected_and_gives_the_volumes_of_the_utm_river` | 23 s | Long run on the full river or DEM in a debug build. |
| owd-format::sections: `the_long_section_sums_to_the_volumes_the_table_shows` | 23 s | Long run on the full river or DEM in a debug build. |
| owd-format::sections: `a_cross_section_is_the_walk_s_station_and_its_cut_agrees_with_the_long_section` | 22 s | Long run on the full river or DEM in a debug build. |
| owd-format::sections: `a_point_picked_on_the_map_gives_the_nearest_station` | 22 s | Long run on the full river or DEM in a debug build. |
| owd-format::volumes: `volumes_needs_a_water_level_and_a_fresh_alignment` | 22 s | Long run on the full river or DEM in a debug build. |
| owd-format::volumes: `volumes_refuses_without_a_dem_and_names_the_dem_it_cannot_read` | 21 s | Long run on the full river or DEM in a debug build. |
| owd-format::alignment: `two_methods_give_two_options_and_a_rerun_replaces_only_its_own` | 20 s | Long run on the full river or DEM in a debug build. |

Times are from one local nextest run (10 cores, tests in parallel, so inflated); CI times are in the pull request.
Python tests (about 33 s in total) all still run in the pull-request job.
