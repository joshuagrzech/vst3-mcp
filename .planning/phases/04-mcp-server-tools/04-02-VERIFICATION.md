# Verification Plan: Batch Tools & Parameter Improvements

**Objective:** Verify the new batch lookup tools, parameter search accuracy improvements, step label introspection, and patch state inspection features.

## 1. Batch Parameter Lookup (`get_params_by_name`)

**Goal:** Confirm that multiple parameter IDs can be retrieved in a single call.

**Steps:**
1.  Load a plugin (e.g., Vital or Serum, if available).
2.  Call `get_params_by_name` with `["Filter Cutoff", "Filter Resonance", "Master Volume"]`.
3.  **Verify:** The response contains an array of results, each with a `match` object containing the correct `id` and `value`.
4.  **Verify:** Fuzzy matching works (e.g., "filt cut" finds "Filter Cutoff").

## 2. Search Accuracy (`find_vst_parameter`)

**Goal:** Confirm exact matches are prioritized.

**Steps:**
1.  Call `find_vst_parameter` with an exact parameter name (e.g., "Attack 1").
2.  **Verify:** The exact match is the first result with a significantly higher score than partial matches.
3.  **Verify:** Partial matches (e.g., "Env 1 Attack") still appear but lower in rank.

## 3. Step Labels (`get_param_info`)

**Goal:** Confirm step labels are returned for enumerated parameters.

**Steps:**
1.  Identify an enumerated parameter (e.g., "Filter Type" or "LFO Shape").
2.  Call `get_param_info` for its ID.
3.  **Verify:** The response includes a `step_labels` array (e.g., `["Sine", "Triangle", "Saw"]`).
4.  **Verify:** `step_count` matches the length of the labels array.

## 4. Patch State Inspection (`get_current_patch_state`)

**Goal:** Confirm non-default parameters can be retrieved.

**Steps:**
1.  Load a plugin and modify a few parameters away from their default values.
2.  Call `get_current_patch_state` (router) or `get_patch_state` (wrapper).
3.  **Verify:** The response lists only the modified parameters.
4.  **Verify:** The values match the current state.

## 5. Batch Set Feedback (`batch_set_realtime`)

**Goal:** Confirm detailed feedback is returned after setting parameters.

**Steps:**
1.  Call `batch_set_realtime` with a mix of valid and out-of-range values.
2.  **Verify:** The response includes a `results` array.
3.  **Verify:** Out-of-range values are marked as `clamped: true` and show the applied value.
4.  **Verify:** Valid values show `clamped: false` and the correct applied value.

## 6. Documentation Tags

**Goal:** Confirm tag-based boosting in search results.

**Steps:**
1.  Add a test documentation file with `<!-- tags: test_tag -->`.
2.  Search for "test_tag".
3.  **Verify:** The file with the tag is returned as the top result, even if the text match is weak.

## 7. Router Integration

**Goal:** Confirm all new tools are exposed correctly via the router.

**Steps:**
1.  Start the router and wrapper.
2.  Call `list_tools` on the router.
3.  **Verify:** `get_params_by_name`, `get_current_patch_state` are listed.
4.  Execute tool calls through the router and verify they proxy correctly to the wrapper.
