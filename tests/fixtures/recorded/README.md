# Recorded runs

Each directory here holds one run of `columbina run` against a real model service.
The test in `tests/replay` plays the run again without the service. The replies
come from the recording, and the log that results must equal the recorded one.

| File | What it is |
|---|---|
| `session.v1.jsonl` | The session log that the run left behind |
| `work/` | The working directory as it was before the run |
| `requests.txt` | What the model was sent, as `columbina.llm.render_requests` writes it |

## Making a recording

1. Copy `work/` to a place of its own and run the task there. The key of the
   service is read from the environment. It is never written to the log.

   ```
   columbina run "TASK" --cwd COPY --session-id NAME
   ```

2. Copy `session.v1.jsonl` from `~/.columbina/sessions/<workspace>/NAME/` into the
   directory of the recording.
3. Replace the working directory in the first line of that file with `/work`.
   The real path names the machine and its user. Nothing else is changed.
4. Run the replay test once with `COLUMBINA_UPDATE_SNAPSHOTS=1` to write
   `requests.txt`, and read the file before committing it.
