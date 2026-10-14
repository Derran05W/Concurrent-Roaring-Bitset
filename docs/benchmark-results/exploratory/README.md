# Exploratory measurements

These pilots guided the implementation choice and are not scored before/after evidence.
`external-before` predates the direct lookup; compiler work overlapped part of that exploratory
run. `external-speculative-index` tried a speculative offset lookup before binary search.
`external-span-pilot` uses the final contiguous-span proof. Trial counts and selected candidates
differ. Do not combine these files with the final five-trial external comparison or use them to
claim a controlled causal speedup. Source hashes and working-tree patches describe each pilot.
