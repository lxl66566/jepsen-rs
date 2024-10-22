use std::{collections::HashMap, sync::Mutex};

use anyhow::Result;
use jepsen_rs::{
    checker::ValidType,
    client::{Client, ElleRwClusterClient, JepsenClient, NemesisClusterClient},
    generator::{
        controller::GeneratorGroupStrategy, elle_rw::ElleRwGenerator, GeneratorGroup,
        NemesisRawGenWrapper,
    },
    nemesis::{NemesisType, ServerId},
    op::{nemesis::OpOrNemesis, Op},
};
use log::{info, LevelFilter};
use madsim::runtime::NodeHandle;

/// Mock cluster
#[derive(Default)]
pub struct TestCluster {
    db: Mutex<HashMap<u64, u64>>,
    size: usize,
    /// In TestCluster, if nemesis_num > quorum, the get/put operation will
    /// fail.
    nemesis_num: usize,
    /// Node Handles for nemeses
    handles: Vec<NodeHandle>,
}

impl TestCluster {
    /// Create a new TestCluster without NodeHandles.
    pub fn new() -> Self {
        Self {
            db: HashMap::new().into(),
            size: 5,
            nemesis_num: 0,
            handles: vec![],
        }
    }

    /// Create a new TestCluster with NodeHandles. Used for Nemesis test.
    pub async fn new_with_handles() -> Self {
        let size = 5;
        let handle = madsim::runtime::Handle::current();
        let mut handles = vec![];
        for x in 0..size {
            handles.push(
                handle
                    .create_node()
                    .name(x.to_string())
                    .ip(format!("192.168.1.{}", x + 1).parse().unwrap())
                    .init(|| async {})
                    .build(),
            )
        }
        Self {
            db: HashMap::new().into(),
            size,
            nemesis_num: 0,
            handles,
        }
    }
    #[inline]
    pub fn quorum(&self) -> usize {
        self.size / 2 + 1
    }
}

/// Accept a get/put/txn operation.
#[async_trait::async_trait]
impl ElleRwClusterClient for TestCluster {
    async fn get(&self, key: u64) -> Result<Option<u64>, String> {
        if self.nemesis_num > self.quorum() {
            return Err("nemesis_num > quorum".to_string());
        }
        Ok(self.db.lock().unwrap().get(&key).cloned())
    }
    async fn put(&self, key: u64, value: u64) -> Result<(), String> {
        if self.nemesis_num > self.quorum() {
            return Err("nemesis_num > quorum".to_string());
        }
        self.db.lock().unwrap().insert(key, value);
        Ok(())
    }
    /// A txn operation should only contains read/write operations.
    async fn txn(&self, mut ops: Vec<Op>) -> Result<Vec<Op>, String> {
        if self.nemesis_num > self.quorum() {
            return Err("nemesis_num > quorum".to_string());
        }
        let mut lock = self.db.lock().unwrap();
        for op in ops.iter_mut() {
            match op {
                Op::Read(key, value) => {
                    *value = lock.get(key).cloned();
                }
                Op::Write(key, value) => {
                    lock.insert(*key, *value);
                }
                _ => {
                    return Err(
                        "txn cannot be in txn, otherwise there will be a deadlock".to_string()
                    );
                }
            }
        }
        Ok(ops)
    }
}

#[async_trait::async_trait]
impl NemesisClusterClient for TestCluster {
    async fn get_all_nodes_handle(&self) -> Vec<NodeHandle> {
        self.handles.clone()
    }
    async fn get_leader_without_term(&self) -> ServerId {
        0
    }
    fn size(&self) -> usize {
        self.size
    }
}

#[test]
pub fn intergration_test_without_nemesis() -> Result<()> {
    _ = pretty_env_logger::formatted_builder()
        .filter_level(log::LevelFilter::Debug)
        .format_timestamp_millis()
        .filter_module("j4rs", LevelFilter::Info)
        .parse_default_env()
        .try_init();
    let mut rt = madsim::runtime::Runtime::new();
    rt.set_allow_system_thread(true); // needed by j4rs

    let cluster = TestCluster::new();
    let raw_gen = ElleRwGenerator::new()?;
    let client = JepsenClient::new(cluster, NemesisRawGenWrapper(Box::new(raw_gen)));
    let client = Box::leak(client.into());
    info!("intergration_test: client created");

    rt.block_on(async move {
        // get generators, transform and merge them
        let g1 = client
            .new_generator(100)
            .filter(|o| matches!(o, OpOrNemesis::Op(Op::Txn(txn)) if txn.len() == 1))
            .await;
        let g2 = client.new_generator(50);
        let g3 = client.new_generator(50);
        info!("intergration_test: generators created");
        let gen_g = GeneratorGroup::new([g1, g2, g3])
            .with_strategy(GeneratorGroupStrategy::RoundRobin(usize::MAX));
        info!("generator group created");
        let res = client.run(gen_g).await.unwrap_or_else(|e| panic!("{}", e));
        info!("history checked result: {:?}", res);
        assert!(matches!(res.valid, ValidType::True));
    });
    Ok(())
}

#[test]
fn intergration_test_with_nemesis() -> Result<()> {
    _ = pretty_env_logger::formatted_builder()
        .filter_level(log::LevelFilter::Debug)
        .format_timestamp_millis()
        .filter_module("j4rs", LevelFilter::Info)
        .parse_default_env()
        .try_init();
    let mut rt = madsim::runtime::Runtime::new();
    rt.set_allow_system_thread(true); // needed by j4rs

    let cluster = rt.block_on(async move { TestCluster::new_with_handles().await });
    let raw_gen = ElleRwGenerator::new()?;
    let client = JepsenClient::new(cluster, NemesisRawGenWrapper(Box::new(raw_gen)));
    let client = Box::leak(client.into());
    info!("intergration_test: client created");

    rt.block_on(async move {
        // get generators, transform and merge them
        let g1 = client
            .new_generator(100)
            .filter(|o| matches!(o, OpOrNemesis::Op(Op::Txn(txn)) if txn.len() == 1))
            .await;
        let g2 = client.new_generator(50);
        let g3 = client.new_generator(50);
        let ng = client.new_nemeses([NemesisType::Kill([1, 2].into_iter().collect())]);
        info!("intergration_test: generators created");
        let gen_g = GeneratorGroup::new_with_count([(g1, 30), (g2, 20), (g3, 20), (ng, 1)])
            .with_strategy(GeneratorGroupStrategy::RoundRobin(usize::MAX));
        info!("generator group created");
        let res = client.run(gen_g).await.unwrap_or_else(|e| panic!("{}", e));
        info!("history checked result: {:?}", res);
        assert!(matches!(res.valid, ValidType::True));
    });
    Ok(())
}
