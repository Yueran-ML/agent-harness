# The tables of the benchmark

These tables were made by `python -m bench table` from the lines of results of the runs of
2026-10-14, with the tasks and the program of commit `149822e`, and judged by the judge of
commit `55e2c18`. How the benchmark works is told in the
[guide](../guide.md#the-benchmark).

A rate of success is given as passed over attempted, the percentage, and its interval of
Wilson at 95 percent in brackets. Tokens are the median of one attempt: all that it put in,
read from a cache or not, and what the model gave out. The share from the cache is of all
the tokens that the attempts of a row put in.

## Acceptance

Each task three times with each model, the whole program on, the commands of the model in a
container of `python:3.12-slim`.

| Model | code | files | sql | compute | All | Input tokens | Output tokens | From cache |
|---|---|---|---|---|---|---|---|---|
| deepseek/deepseek-flash | 45/45 = 100% [92%, 100%] | 36/36 = 100% [90%, 100%] | 45/45 = 100% [92%, 100%] | 90/90 = 100% [96%, 100%] | 216/216 = 100% [98%, 100%] | 16790 | 589 | 94% |
| deepseek/deepseek-v4-pro | 43/45 = 96% [85%, 99%] | 36/36 = 100% [90%, 100%] | 45/45 = 100% [92%, 100%] | 90/90 = 100% [96%, 100%] | 214/216 = 99% [97%, 100%] | 16856 | 544 | 95% |

`deepseek-flash` passed every attempt. `deepseek-v4-pro` failed two of its three attempts at
one task, `code-word-count-contractions`: its fix split the words of a text at every letter
outside English, so that `café` became `caf`, which the hidden tests do not let pass.

The first judging of the runs counted two more of its attempts as failed. Their queries
wrote an amount with `printf('%.2f', …)`, as text, where the question asked for it rounded to
two places, and the judge compared that text with a number. The judge was set right, and
`python -m bench judge` judged every attempt of the three runs again from what it left. No
other verdict changed.

## The parts of the program

Each part switched off in turn and set against the whole program, with `deepseek-flash`,
each task three times in each variant. The program is told that the window of the model is
16,384 tokens, the window of the local models, so that the parts that keep a conversation
small have something to do. The service takes far more, so a conversation that is not made
smaller still fits it: what the table shows of those parts is what they save and what they
cost, and not what they rescue where the window is that small. The difference is the rate
of the variant less that of the whole program, with its interval of Newcombe.

| Variant | What is switched off |
|---|---|
| `no-compaction` | The summary of the oldest part of a conversation as it nears the window |
| `no-todo` | The list of things to do that the model keeps for itself |
| `no-spill` | The limit on the size of a result: a larger one goes into the conversation whole, where it would have been cut to its ends and saved |
| `no-conduct` | The rules of conduct of the system prompt |

| Variant | All | Difference | Input tokens | Output tokens |
|---|---|---|---|---|
| all | 215/216 = 100% [97%, 100%] | | 16837 | 575 |
| no-compaction | 216/216 = 100% [98%, 100%] | +0% [-1%, +3%] | 16909 | 564 |
| no-conduct | 216/216 = 100% [98%, 100%] | +0% [-1%, +3%] | 11382 | 580 |
| no-spill | 216/216 = 100% [98%, 100%] | +0% [-1%, +3%] | 16867 | 567 |
| no-todo | 216/216 = 100% [98%, 100%] | +0% [-1%, +3%] | 15582 | 525 |

No part changes whether `deepseek-flash` succeeds at these tasks, and what each part costs
shows in the tokens. The rules of conduct are about a third of the input of a median
attempt, and the list of things to do about 7 percent; the service read 93 percent of all
the input of this run from its cache, at a fiftieth of the price. The compaction and the
limit on results cost nothing while they have nothing to do, and here they had almost
nothing to do: no conversation came near the window, and results were cut short in one
attempt of the 1,080. The one attempt of the whole program that failed worked its answer
out and said it, and did not submit it.

## Local models

Twenty of the tasks, five of each category, once with each model, at a window of 16,384
tokens; a model is marked usable where it passes half of its attempts or more. Each model was
served by an Ollama of the benchmark's own that holds one request to one model, on a laptop
with a GeForce RTX 4080 of 12 GB. Where the card had room, two of these ran beside each
other, each with its own Ollama; that changes how long a run takes, and nothing else. The
twelve runs took two hours and twenty minutes, from 2.5 to 49.5 minutes a model. The names
are those under which the models lie on that machine, and their ends say the quantization:
`q2k` is Q2_K, `q3km` Q3_K_M, `q3kl` Q3_K_L, `q4km` Q4_K_M, `q8` Q8_0, and `fp16` the weights
in 16 bits.

| Model | code | files | sql | compute | All | Usable |
|---|---|---|---|---|---|---|
| ollama/llama3.1-8b-q3kl | 0/5 = 0% [0%, 43%] | 0/5 = 0% [0%, 43%] | 3/5 = 60% [23%, 88%] | 0/5 = 0% [0%, 43%] | 3/20 = 15% [5%, 36%] | no |
| ollama/llama3.1-8b-q4km | 0/5 = 0% [0%, 43%] | 0/5 = 0% [0%, 43%] | 1/5 = 20% [4%, 62%] | 0/5 = 0% [0%, 43%] | 1/20 = 5% [1%, 24%] | no |
| ollama/llama3.1-8b-q8 | 0/5 = 0% [0%, 43%] | 0/5 = 0% [0%, 43%] | 3/5 = 60% [23%, 88%] | 0/5 = 0% [0%, 43%] | 3/20 = 15% [5%, 36%] | no |
| ollama/llama3.2-3b-mz-q2k | 0/5 = 0% [0%, 43%] | 0/5 = 0% [0%, 43%] | 1/5 = 20% [4%, 62%] | 0/5 = 0% [0%, 43%] | 1/20 = 5% [1%, 24%] | no |
| ollama/llama3.2-3b-q4km | 0/5 = 0% [0%, 43%] | 0/5 = 0% [0%, 43%] | 2/5 = 40% [12%, 77%] | 0/5 = 0% [0%, 43%] | 2/20 = 10% [3%, 30%] | no |
| ollama/llama3.2:3b-instruct-fp16 | 0/5 = 0% [0%, 43%] | 0/5 = 0% [0%, 43%] | 1/5 = 20% [4%, 62%] | 0/5 = 0% [0%, 43%] | 1/20 = 5% [1%, 24%] | no |
| ollama/qwen2.5-3b-q2k | 0/5 = 0% [0%, 43%] | 0/5 = 0% [0%, 43%] | 1/5 = 20% [4%, 62%] | 0/5 = 0% [0%, 43%] | 1/20 = 5% [1%, 24%] | no |
| ollama/qwen2.5-3b-q4km | 0/5 = 0% [0%, 43%] | 0/5 = 0% [0%, 43%] | 3/5 = 60% [23%, 88%] | 0/5 = 0% [0%, 43%] | 3/20 = 15% [5%, 36%] | no |
| ollama/qwen2.5-7b-q3km | 0/5 = 0% [0%, 43%] | 0/5 = 0% [0%, 43%] | 1/5 = 20% [4%, 62%] | 4/5 = 80% [38%, 96%] | 5/20 = 25% [11%, 47%] | no |
| ollama/qwen2.5-7b-q4km | 0/5 = 0% [0%, 43%] | 0/5 = 0% [0%, 43%] | 0/5 = 0% [0%, 43%] | 1/5 = 20% [4%, 62%] | 1/20 = 5% [1%, 24%] | no |
| ollama/qwen2.5-7b-q8 | 0/5 = 0% [0%, 43%] | 0/5 = 0% [0%, 43%] | 3/5 = 60% [23%, 88%] | 3/5 = 60% [23%, 88%] | 6/20 = 30% [15%, 52%] | no |
| ollama/qwen2.5:3b-instruct-fp16 | 0/5 = 0% [0%, 43%] | 0/5 = 0% [0%, 43%] | 1/5 = 20% [4%, 62%] | 0/5 = 0% [0%, 43%] | 1/20 = 5% [1%, 24%] | no |

None of the twelve is usable. The best is Qwen2.5 7B in eight bits, with 6 of 20, where
`deepseek-flash` passes all twenty of the same tasks. No model passes a task of code or of
files: what passes are questions about the database and reckonings. These are small
instruction models of 2024, of 3 to 8 billion parameters, and the models that are trained
for long work with tools came after them. So the table says what such models can do with
the program on one machine, and where they fail. More bits do not show in it: within a
family, the quantizations lie inside one another's intervals.

Each model made one attempt at each task, with the sampling that its model file sets, so a
second run comes out otherwise. A run of the first two models a day earlier, broken off when
the machine froze, passed 4 and 2 of the same five tasks of SQL, where this one passed 0 and
1. The two runs sent the same prompt and the same tools, and their records part where the
model answers otherwise. The intervals of the table are wide for that reason.

What became of the 240 attempts, from their records:

| What happened | Attempts |
|---|---|
| Passed | 28 |
| The task was done wrong: code that the hidden tests fail, files that hold the wrong thing, a wrong number or wrong rows | 114 |
| Nothing was handed in: the model found what was asked and wrote it out, without the tool that submits it | 29 |
| The service could not take the reply: it refused the call that the model wrote, or the stream broke off | 34 |
| No tool was called at all | 17 |
| Out of steps | 12 |
| Out of time | 6 |

Three things stand out in the records.

- **Calls that the server cannot read.** Llama wrote calls with a line break inside a string
  of JSON, which the server of Ollama refused (`llama-server returned invalid tool call
  arguments for "edit": invalid character '\n' in string literal`), or its stream broke off
  before a reply was whole. The retries met the same, and the turn ended with an error. That
  ended 34 of the 120 attempts of Llama, and none of Qwen.
- **An answer found, and not handed in.** The tasks of SQL ask for the query to be handed in
  with `submit_sql`, and those of reckoning for the answer with `submit_answer`. In 29
  attempts the model found what was asked and wrote it out in its answer instead.
- **Calls made blindly, and again and again.** The checks of the program refused 51 edits of
  files that the model had not read, 102 calls whose arguments did not fit the tool, and 32
  calls of skills and kinds of subagent that are not there; each refusal went back to the
  model as the result of its call. 33 of the attempts made one call three times or more, and
  one reply held the same `read` 97 times.

## Cost

At the prices that DeepSeek gives for its models
([pricing](https://api-docs.deepseek.com/quick_start/pricing), read on 2026-10-14), in US
dollars, at the off-peak price at which the runs were made:

| Run | Attempts | Cost |
|---|---|---|
| Acceptance, `deepseek-flash` | 216 | 0.17 |
| Acceptance, `deepseek-v4-pro` | 216 | 0.60 |
| The parts of the program, `deepseek-flash` | 1,080 | 0.90 |

## Time and memory

What section 14.4 of the design asks for, measured on the machine of the author (Windows 11,
Python 3.11.9), the median of five runs. `tests/test_performance.py` holds the same
measurements to loose budgets in CI.

| Measurement | Result |
|---|---|
| Composing the program, `columbina run --dump-config` | 0.96 s |
| One task with a scripted model | 1.14 s |
| A session of 1,000 steps, on disk | 3.9 MB |
| Taking it up again | 0.18 s |
| Adding up its tokens | 9 ms |
| Memory at the peak while it is taken up | 21 MB |
