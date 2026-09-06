/*
Copyright (C) 2022 Axel Gneiting
Copyright (C) 2026 vkqr-engine contributors

This program is free software; you can redistribute it and/or
modify it under the terms of the GNU General Public License
as published by the Free Software Foundation; either version 2
of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.

See the GNU General Public License for more details.

You should have received a copy of the GNU General Public License
along with this program; if not, write to the Free Software
Foundation, Inc., 59 Temple Place - Suite 330, Boston, MA  02111-1307, USA.

*/
// tasks_glue.c -- the C remainder of tasks.c under -Duse_rust_tasks
//
// Compiled instead of tasks.c when the Rust scheduler (quake-tasks via
// quake-capi, Rust migration Phase 8 M2, ADR-016) provides the Task_*/Tasks_*
// ABI. Only the _DEBUG `test_tasks` command stays here, verbatim: host.c and
// host_glue.c both register TestTasks_f, and driving the Rust scheduler
// through the real C ABI is exactly what that stress test is for.
#include "quakedef.h"

#ifdef _DEBUG
/*
=================
TASKS_TEST_ASSERT
=================
*/
#define TASKS_TEST_ASSERT(cond, what) \
	if (!(cond))                      \
	{                                 \
		Con_Printf ("%s\n", what);    \
		abort ();                     \
	}

/*
=================
LotsOfTasks
=================
*/
static void LotsOfTasksTestTask (void *counters_ptr)
{
	uint32_t *counters = *((uint32_t **)counters_ptr);
	++counters[Tasks_GetWorkerIndex ()];
}
static void LotsOfTasks (void)
{
	static const int NUM_TASKS = 100000;
	TEMP_ALLOC_ZEROED (uint32_t, counters, TASKS_MAX_WORKERS);
	TEMP_ALLOC (task_handle_t, handles, NUM_TASKS);
	for (int i = 0; i < NUM_TASKS; ++i)
		handles[i] = Task_AllocateAssignFuncAndSubmit (LotsOfTasksTestTask, (void *)&counters, sizeof (uint32_t *));
	for (int i = 0; i < NUM_TASKS; ++i)
		Task_Join (handles[i], TASK_TIMEOUT_INFINITE);
	uint32_t counters_sum = 0;
	for (int i = 0; i < TASKS_MAX_WORKERS; ++i)
		counters_sum += counters[i];
	TASKS_TEST_ASSERT (counters_sum == NUM_TASKS, "Wrong counters_sum");
	TEMP_FREE (handles);
	TEMP_FREE (counters);
}

/*
=================
IndexedTasks
=================
*/
static void IndexedTestTask (int index, void *counters_ptr)
{
	uint32_t *counters = *((uint32_t **)counters_ptr);
	++counters[Tasks_GetWorkerIndex ()];
}
static void IndexedTasks ()
{
	static const int LIMIT = 100000;
	TEMP_ALLOC_ZEROED (uint32_t, counters, TASKS_MAX_WORKERS);
	task_handle_t task = Task_AllocateAssignIndexedFuncAndSubmit (IndexedTestTask, LIMIT, (void *)&counters, sizeof (uint32_t *));
	Task_Join (task, TASK_TIMEOUT_INFINITE);
	uint32_t counters_sum = 0;
	for (int i = 0; i < TASKS_MAX_WORKERS; ++i)
		counters_sum += counters[i];
	TASKS_TEST_ASSERT (counters_sum == LIMIT, "Wrong counters_sum");
	TEMP_FREE (counters);
}

/*
=================
TestTasks_f
=================
*/
void TestTasks_f (void)
{
	LotsOfTasks ();
	IndexedTasks ();
}
#endif
