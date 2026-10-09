# Recorded runs

Each directory here holds one run of `columbina run` against a real model service.
The test in `tests/replay` plays the run again without the service. The replies
come from the recording, the tools run again in a copy of the working directory,
and the log that results must equal the recorded one.

| File | What it is |
|---|---|
| `session.v1.jsonl` | The session log that the run left behind |
| `work/` | The working directory as it was before the run |
| `recording.yml` | The plugins that the run had, and the platform if the run is bound to one |
| `requests.txt` | What the model was sent, as `columbina.llm.render_requests` writes it |

| Recording | What happens in it |
|---|---|
| `deepseek-read` | A question is answered from a file. One call of `read` |
| `deepseek-fix` | A calculator with a mistake is mended and checked. Two calls of `read`, one of `edit`, one of the shell tool |

A run that calls the shell tool names the shell of the platform it was made on,
`powershell` or `bash`. Such a run can happen again on that platform only, and
`recording.yml` says which one it is.

## Making a recording

1. Copy `work/` to a place of its own and run the task there, with a data
   directory that holds no rules and no instructions of yours. The key of the
   service is read from the environment. It is never written to the log.

   ```
   columbina run "TASK" --cwd COPY --session-id NAME
   ```

   Set `PYTHONDONTWRITEBYTECODE=1` if the run executes Python, so that the
   working directory holds the same files when the run happens again.

2. Copy `session.v1.jsonl` from `<COLUMBINA_HOME>/sessions/<workspace>/NAME/` into
   the directory of the recording.
3. Replace the working directory with `/work` wherever that file names it: in
   the first line, and in the facts about the run that the model was told. The
   real path names the machine and its user. Nothing else is changed.
4. Write `recording.yml`: the profile is the bundle that the run loaded, with
   the rules that the command line gave.
5. Run the replay test once with `COLUMBINA_UPDATE_SNAPSHOTS=1` to write
   `requests.txt`, and read both files before committing them.
