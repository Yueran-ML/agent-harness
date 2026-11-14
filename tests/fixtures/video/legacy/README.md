# Legacy migration fixtures

Captured from the real legacy Editor at local commit a315ec7, before its retirement.
`test_migrate.py::old_project` produced Old Film; the audio-only timebase test produced Audio.
Both passed the Task 22 real-engine migration and undo checks before capture.

Each JSON maps an old relative filename to its exact text. Only source paths in project.json
were shortened to filenames for portability. The test installer substitutes freshly generated
source identities in the manifest, transcript and silence-cache filenames; it does not rebuild
old timelines with current editing code. Derived WAV caches are omitted so migration can also
exercise their absence. History and story files are retained to verify preservation.
