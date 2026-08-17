/*
Copyright (C) 2026 vkQuake contributors

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

#ifndef __Q_THREAD_H
#define __Q_THREAD_H

// q_thread.h -- opaque threading primitives so engine code and core headers
// need no SDL includes (Rust migration Phase 0, PLAN.md 4.1). The only
// implementation is SDL (q_thread_sdl.c); the wrappers are thin call-throughs
// with no semantic change.

#include "q_types.h"

typedef struct qmutex_s	 qmutex_t;
typedef struct qcond_s	 qcond_t;
typedef struct qsem_s	 qsem_t;
typedef struct qthread_s qthread_t;

qmutex_t *QMutex_Create (void);
void	  QMutex_Destroy (qmutex_t *mutex);
void	  QMutex_Lock (qmutex_t *mutex);
void	  QMutex_Unlock (qmutex_t *mutex);

qcond_t *QCond_Create (void);
void	 QCond_Destroy (qcond_t *cond);
void	 QCond_Wait (qcond_t *cond, qmutex_t *mutex);
qboolean QCond_WaitTimeout (qcond_t *cond, qmutex_t *mutex, uint32_t timeout_ms); /* true = signaled, false = timed out */
void	 QCond_Signal (qcond_t *cond);
void	 QCond_Broadcast (qcond_t *cond);

qsem_t	*QSem_Create (uint32_t initial_value);
void	 QSem_Destroy (qsem_t *sem);
void	 QSem_Wait (qsem_t *sem);
qboolean QSem_TryWait (qsem_t *sem); /* true = acquired */
void	 QSem_Post (qsem_t *sem);

typedef int (*qthread_func_t) (void *data);
qthread_t *QThread_Create (qthread_func_t fn, const char *name, void *data);
void	   QThread_Detach (qthread_t *thread);
void	   QThread_Wait (qthread_t *thread); /* join; frees the thread handle */

int QThread_NumLogicalCores (void);

#endif /* __Q_THREAD_H */
