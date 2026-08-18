/*
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

// q_thread_sdl.c -- SDL implementation of the q_thread.h primitives.
// The SDL2 name-compat mappings that used to live in quakedef.h are private
// to this file now; engine code uses the q_thread API exclusively.

#include "q_stdinc.h"
#include "q_thread.h"

#ifndef USE_SDL3
#define SDL_Mutex SDL_mutex

#define SDL_Condition							SDL_cond
#define SDL_CreateCondition						SDL_CreateCond
#define SDL_DestroyCondition					SDL_DestroyCond
#define SDL_SignalCondition						SDL_CondSignal
#define SDL_BroadcastCondition					SDL_CondBroadcast
#define SDL_WaitCondition						SDL_CondWait
#define SDL_WaitConditionTimeout(cond, mtx, ms) (SDL_CondWaitTimeout (cond, mtx, ms) == 0)

#define SDL_SignalSemaphore		  SDL_SemPost
#define SDL_Semaphore			  SDL_sem
#define SDL_TryWaitSemaphore(sem) (SDL_SemTryWait (sem) == 0)
#define SDL_WaitSemaphore		  SDL_SemWait

#define SDL_GetNumLogicalCPUCores SDL_GetCPUCount
#endif

qmutex_t *QMutex_Create (void)
{
	return (qmutex_t *)SDL_CreateMutex ();
}

void QMutex_Destroy (qmutex_t *mutex)
{
	SDL_DestroyMutex ((SDL_Mutex *)mutex);
}

void QMutex_Lock (qmutex_t *mutex)
{
	SDL_LockMutex ((SDL_Mutex *)mutex);
}

void QMutex_Unlock (qmutex_t *mutex)
{
	SDL_UnlockMutex ((SDL_Mutex *)mutex);
}

qcond_t *QCond_Create (void)
{
	return (qcond_t *)SDL_CreateCondition ();
}

void QCond_Destroy (qcond_t *cond)
{
	SDL_DestroyCondition ((SDL_Condition *)cond);
}

void QCond_Wait (qcond_t *cond, qmutex_t *mutex)
{
	SDL_WaitCondition ((SDL_Condition *)cond, (SDL_Mutex *)mutex);
}

qboolean QCond_WaitTimeout (qcond_t *cond, qmutex_t *mutex, uint32_t timeout_ms)
{
	return SDL_WaitConditionTimeout ((SDL_Condition *)cond, (SDL_Mutex *)mutex, timeout_ms);
}

void QCond_Signal (qcond_t *cond)
{
	SDL_SignalCondition ((SDL_Condition *)cond);
}

void QCond_Broadcast (qcond_t *cond)
{
	SDL_BroadcastCondition ((SDL_Condition *)cond);
}

qsem_t *QSem_Create (uint32_t initial_value)
{
	return (qsem_t *)SDL_CreateSemaphore (initial_value);
}

void QSem_Destroy (qsem_t *sem)
{
	SDL_DestroySemaphore ((SDL_Semaphore *)sem);
}

void QSem_Wait (qsem_t *sem)
{
	SDL_WaitSemaphore ((SDL_Semaphore *)sem);
}

qboolean QSem_TryWait (qsem_t *sem)
{
	return SDL_TryWaitSemaphore ((SDL_Semaphore *)sem);
}

void QSem_Post (qsem_t *sem)
{
	SDL_SignalSemaphore ((SDL_Semaphore *)sem);
}

qthread_t *QThread_Create (qthread_func_t fn, const char *name, void *data)
{
	return (qthread_t *)SDL_CreateThread (fn, name, data);
}

void QThread_Detach (qthread_t *thread)
{
	SDL_DetachThread ((SDL_Thread *)thread);
}

void QThread_Wait (qthread_t *thread)
{
	SDL_WaitThread ((SDL_Thread *)thread, NULL);
}

int QThread_NumLogicalCores (void)
{
	return SDL_GetNumLogicalCPUCores ();
}
