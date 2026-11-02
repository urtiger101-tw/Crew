# Crew project collaboration preferences

- For implementation, debugging, and other work in this project that needs collaboration, dispatch **GPT-6.1 Sol xhigh** and **GPT-6.1 Terra xhigh** by default to assist the lead agent (主代理).
- Tool settings use `model: "gpt-6.1-sol"` and `model: "gpt-6.1-terra"` respectively. Both set `reasoning_effort: "xhigh"`. If a specified model is unavailable, report (回報) it explicitly first. Do not silently substitute the model or the reasoning level.
- Assign only work that is necessary, independent, and has clear file ownership (所有權). The lead agent handles integration, fixes, and final verification. Avoid duplicate implementation, duplicate review (審查), and overwriting the changes of other agents.
- An author must not review their own change (作者不得自審). When an independent review (獨立審查) is needed, have an agent that did not write that part perform it.
- Keep to the current task's authorization (授權) boundaries for testing, commit (提交), push (推送), and restart (重啟). The collaboration preferences above do not, by themselves, authorize restarting services or changing global settings.
