//! 蜂群消息总线与队列抽象 (Swarm Message Bus & Queue Abstraction)
//! 提供本地高效队列 (LocalQueueBus) 与分布式中间件适配能力 (RabbitMQ / Redis PubSub)，
//! 支持点对点任务分发 (Competing Consumers)、主题扇出广播 (Topic Pub/Sub)、
//! 任务租约超时机制与死信队列 (DLQ)。

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use super::swarm_channel::ChannelMessage;

/// 蜂群工作任务单元（用于点对点分发与竞争消费）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmTaskItem {
    pub task_id: String,
    pub queue: String,
    pub target_role: String,
    pub payload: serde_json::Value,
    pub priority: u8,
    pub attempts: u32,
    pub max_retries: u32,
    pub created_at: DateTime<Utc>,
    pub leased_at: Option<DateTime<Utc>>,
    pub lease_expire_at: Option<DateTime<Utc>>,
}

impl SwarmTaskItem {
    pub fn new(queue: impl Into<String>, target_role: impl Into<String>, payload: serde_json::Value) -> Self {
        Self {
            task_id: format!("tsk-{}", &uuid::Uuid::new_v4().to_string()[..8]),
            queue: queue.into(),
            target_role: target_role.into(),
            payload,
            priority: 5,
            attempts: 0,
            max_retries: 3,
            created_at: Utc::now(),
            leased_at: None,
            lease_expire_at: None,
        }
    }

    pub fn with_priority(mut self, priority: u8) -> Self {
        self.priority = priority;
        self
    }

    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }
}

/// 统一蜂群消息总线 Trait
#[async_trait::async_trait]
pub trait SwarmMessageBus: Send + Sync {
    /// 消息总线实现类型标识
    fn bus_type(&self) -> &'static str;

    /// 发布信令/事件到指定 topic（用于广播或主题分发）
    async fn publish(&self, topic: &str, message: ChannelMessage) -> Result<(), String>;

    /// 订阅指定 topic 的实时消息流
    fn subscribe(&self, topic: &str) -> broadcast::Receiver<ChannelMessage>;

    /// 将任务投递到角色工作队列（支持优先级与可靠确认）
    async fn enqueue_task(&self, queue: &str, task: SwarmTaskItem) -> Result<String, String>;

    /// 从工作队列拉取待处理任务（自动附带租约锁定，防止并发重入）
    async fn poll_task(&self, queue: &str, lease_secs: u64) -> Result<Option<SwarmTaskItem>, String>;

    /// 确认任务成功处理完毕（从队列中彻底移除）
    async fn ack_task(&self, queue: &str, task_id: &str) -> Result<(), String>;

    /// 拒绝或任务失败（支持自动重试或流入死信队列 DLQ）
    async fn nack_task(&self, queue: &str, task_id: &str, requeue: bool) -> Result<(), String>;

    /// 查询队列当前的死信任务 (DLQ)
    async fn get_dlq(&self, queue: &str) -> Result<Vec<SwarmTaskItem>, String>;

    /// 获取队列状态快照 (等待中、处理中、死信数)
    fn queue_stats(&self, queue: &str) -> (usize, usize, usize);
}

// ─────────────────────────────────────────────────────────────────────────────
// 本地高性能队列实现 (LocalQueueBus: Broadcast + Priority Lease Queue)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Default)]
struct QueueStorage {
    /// 待处理任务 (按优先级降序、创建时间升序排列)
    pending: Vec<SwarmTaskItem>,
    /// 正在被 Worker 执行中（已租约锁定）的任务
    in_flight: HashMap<String, SwarmTaskItem>,
    /// 达到最大重试次数淘汰的死信任务
    dlq: Vec<SwarmTaskItem>,
}

/// 本地脱机自治消息总线（基于 Tokio Broadcast 与内存租约状态机）
pub struct LocalQueueBus {
    /// 主题广播分发通道映射 (topic -> broadcast sender)
    channels: Arc<RwLock<HashMap<String, broadcast::Sender<ChannelMessage>>>>,
    /// 角色任务工作队列映射 (queue_name -> QueueStorage)
    queues: Arc<RwLock<HashMap<String, QueueStorage>>>,
    broadcast_capacity: usize,
}

impl Default for LocalQueueBus {
    fn default() -> Self {
        Self::new(1024)
    }
}

impl LocalQueueBus {
    pub fn new(broadcast_capacity: usize) -> Self {
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
            queues: Arc::new(RwLock::new(HashMap::new())),
            broadcast_capacity,
        }
    }

    fn get_or_create_sender(&self, topic: &str) -> broadcast::Sender<ChannelMessage> {
        let mut map = self.channels.write().unwrap();
        map.entry(topic.to_string())
            .or_insert_with(|| {
                let (tx, _) = broadcast::channel(self.broadcast_capacity);
                tx
            })
            .clone()
    }
}

#[async_trait::async_trait]
impl SwarmMessageBus for LocalQueueBus {
    fn bus_type(&self) -> &'static str {
        "local_queue"
    }

    async fn publish(&self, topic: &str, message: ChannelMessage) -> Result<(), String> {
        let sender = self.get_or_create_sender(topic);
        // 如果当前没有活跃的 receiver，send 会返回 SendError，属于正常广播特性
        let _ = sender.send(message);
        Ok(())
    }

    fn subscribe(&self, topic: &str) -> broadcast::Receiver<ChannelMessage> {
        let sender = self.get_or_create_sender(topic);
        sender.subscribe()
    }

    async fn enqueue_task(&self, queue: &str, task: SwarmTaskItem) -> Result<String, String> {
        let task_id = task.task_id.clone();
        let mut map = self.queues.write().unwrap();
        let storage = map.entry(queue.to_string()).or_default();

        // 插入并维护优先级顺序（高优先级优先，相同优先级 FIFO）
        let pos = storage
            .pending
            .iter()
            .position(|it| it.priority < task.priority)
            .unwrap_or(storage.pending.len());
        storage.pending.insert(pos, task);

        Ok(task_id)
    }

    async fn poll_task(&self, queue: &str, lease_secs: u64) -> Result<Option<SwarmTaskItem>, String> {
        let mut map = self.queues.write().unwrap();
        let storage = map.entry(queue.to_string()).or_default();
        let now = Utc::now();

        // 1. 巡检租约超时的 in_flight 任务，自动回收重入 pending（防止工蜂假死卡死队列）
        let mut expired_keys = Vec::new();
        for (id, item) in storage.in_flight.iter_mut() {
            if let Some(exp) = item.lease_expire_at {
                if now > exp {
                    expired_keys.push(id.clone());
                }
            }
        }

        for id in expired_keys {
            if let Some(mut item) = storage.in_flight.remove(&id) {
                item.leased_at = None;
                item.lease_expire_at = None;
                // 租约超时重新归队
                storage.pending.insert(0, item);
            }
        }

        // 2. 取出下一个可用任务
        if storage.pending.is_empty() {
            return Ok(None);
        }

        let mut item = storage.pending.remove(0);
        item.attempts += 1;
        item.leased_at = Some(now);
        item.lease_expire_at = Some(now + Duration::seconds(lease_secs as i64));

        storage.in_flight.insert(item.task_id.clone(), item.clone());
        Ok(Some(item))
    }

    async fn ack_task(&self, queue: &str, task_id: &str) -> Result<(), String> {
        let mut map = self.queues.write().unwrap();
        let storage = map.entry(queue.to_string()).or_default();

        if storage.in_flight.remove(task_id).is_some() {
            Ok(())
        } else if let Some(idx) = storage.pending.iter().position(|it| it.task_id == task_id) {
            storage.pending.remove(idx);
            Ok(())
        } else {
            Err(format!("Task {task_id} not found in in-flight or pending"))
        }
    }

    async fn nack_task(&self, queue: &str, task_id: &str, requeue: bool) -> Result<(), String> {
        let mut map = self.queues.write().unwrap();
        let storage = map.entry(queue.to_string()).or_default();

        let item = storage.in_flight.remove(task_id).or_else(|| {
            storage
                .pending
                .iter()
                .position(|it| it.task_id == task_id)
                .map(|idx| storage.pending.remove(idx))
        });

        let Some(mut item) = item else {
            return Err(format!("Task {task_id} not found for nack"));
        };

        if requeue && item.attempts < item.max_retries {
            item.leased_at = None;
            item.lease_expire_at = None;
            let pos = storage
                .pending
                .iter()
                .position(|it| it.priority < item.priority)
                .unwrap_or(storage.pending.len());
            storage.pending.insert(pos, item);
        } else {
            // 超过最大重试次数或明确不重新排队，转入死信队列 (DLQ)
            storage.dlq.push(item);
        }

        Ok(())
    }

    async fn get_dlq(&self, queue: &str) -> Result<Vec<SwarmTaskItem>, String> {
        let map = self.queues.read().unwrap();
        let storage = map.get(queue);
        Ok(storage.map(|s| s.dlq.clone()).unwrap_or_default())
    }

    fn queue_stats(&self, queue: &str) -> (usize, usize, usize) {
        let map = self.queues.read().unwrap();
        if let Some(s) = map.get(queue) {
            (s.pending.len(), s.in_flight.len(), s.dlq.len())
        } else {
            (0, 0, 0)
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 分布式 RabbitMQ 拓扑定义与配置规范 (对齐 DeepCloud Spring Cloud 微服务契约)
// ─────────────────────────────────────────────────────────────────────────────

/// RabbitMQ 拓扑规范（与 Java 微服务 application.yml 保持一致）
pub struct RabbitMqSwarmTopology;

impl RabbitMqSwarmTopology {
    /// 蜂群主题交换机 (Topic Exchange) - 适用于事件分流与状态同步
    pub const TOPIC_EXCHANGE: &'static str = "swarm.topic";
    /// 蜂群广播交换机 (Fanout Exchange) - 适用于全员决策、共识与意识同步
    pub const FANOUT_EXCHANGE: &'static str = "swarm.fanout";
    /// 蜂群定向工作交换机 (Direct Exchange) - 适用于指定角色或智能体派工
    pub const DIRECT_EXCHANGE: &'static str = "swarm.direct";

    /// 常用 Routing Key 规划
    pub const RK_PROGRESS: &'static str = "swarm.event.progress";
    pub const RK_BLOCKER: &'static str = "swarm.event.blocker";
    pub const RK_DIRECTIVE: &'static str = "swarm.directive";
    pub const RK_CONSENSUS: &'static str = "swarm.consensus";

    /// 根据角色生成队列名，支持多实例竞抢负载均衡
    pub fn role_queue(role: &str) -> String {
        format!("swarm.worker.{}", role.to_lowercase())
    }

    /// 死信队列名 (DLQ)
    pub fn dlq_queue(queue_name: &str) -> String {
        format!("{}.dlq", queue_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domains::swarm_intelligence::swarm_channel::ChannelMessageType;

    #[tokio::test]
    async fn test_local_queue_publish_and_subscribe() {
        let bus = LocalQueueBus::default();
        let mut rx = bus.subscribe("swarm.blocker");

        let msg = ChannelMessage {
            id: "msg-1".into(),
            sender: "agent-tester".into(),
            target: None,
            msg_type: ChannelMessageType::BlockerAlert,
            summary: "WAF blocked requests".into(),
            payload: serde_json::json!({ "code": 403 }),
            is_urgent: true,
            created_at: Utc::now(),
            consumed_by_brain: false,
        };

        bus.publish("swarm.blocker", msg.clone()).await.unwrap();

        let received = rx.recv().await.unwrap();
        assert_eq!(received.id, "msg-1");
        assert_eq!(received.summary, "WAF blocked requests");
    }

    #[tokio::test]
    async fn test_local_queue_task_lifecycle_and_dlq() {
        let bus = LocalQueueBus::default();
        let q_name = "swarm.worker.coder";

        // 1. 入队高优先级任务
        let task_low = SwarmTaskItem::new(q_name, "coder", serde_json::json!({ "step": 1 })).with_priority(1);
        let task_high = SwarmTaskItem::new(q_name, "coder", serde_json::json!({ "step": 2 })).with_priority(10);

        bus.enqueue_task(q_name, task_low).await.unwrap();
        bus.enqueue_task(q_name, task_high).await.unwrap();

        let (pending, in_flight, dlq) = bus.queue_stats(q_name);
        assert_eq!(pending, 2);
        assert_eq!(in_flight, 0);

        // 2. 拉取任务：高优先级任务应当优先被取出
        let polled = bus.poll_task(q_name, 30).await.unwrap().unwrap();
        assert_eq!(polled.priority, 10);
        assert_eq!(polled.payload["step"], 2);

        // 3. 模拟任务失败 NACK（重新排队）
        bus.nack_task(q_name, &polled.task_id, true).await.unwrap();
        let (pending, in_flight, dlq) = bus.queue_stats(q_name);
        assert_eq!(pending, 2);

        // 4. 重复 NACK 达到最大重试次数，验证转入死信队列 (DLQ)
        let polled2 = bus.poll_task(q_name, 30).await.unwrap().unwrap();
        bus.nack_task(q_name, &polled2.task_id, true).await.unwrap(); // attempt 2
        let polled3 = bus.poll_task(q_name, 30).await.unwrap().unwrap();
        bus.nack_task(q_name, &polled3.task_id, true).await.unwrap(); // attempt 3 (reached max)

        let dlq_items = bus.get_dlq(q_name).await.unwrap();
        assert_eq!(dlq_items.len(), 1);
        assert_eq!(dlq_items[0].task_id, polled.task_id);

        // 5. 拉取剩余的低优先级任务并 ACK 成功
        let polled_low = bus.poll_task(q_name, 30).await.unwrap().unwrap();
        assert_eq!(polled_low.priority, 1);
        bus.ack_task(q_name, &polled_low.task_id).await.unwrap();

        let (pending, in_flight, dlq) = bus.queue_stats(q_name);
        assert_eq!(pending, 0);
        assert_eq!(in_flight, 0);
        assert_eq!(dlq, 1);
    }
}
