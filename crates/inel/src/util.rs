use std::{future::Future, io::Result};

pub async fn chain3<F1, T1, F2, T2, F3, T3>(f1: F1, f2: F2, f3: F3) -> Result<(T1, T2, T3)>
where
    F1: Future<Output = Result<T1>>,
    F2: Future<Output = Result<T2>>,
    F3: Future<Output = Result<T3>>,
{
    let (r1, r2, r3) = futures::future::join3(f1, f2, f3).await;
    let t1 = r1?;
    let t2 = r2?;
    let t3 = r3?;
    Ok((t1, t2, t3))
}
