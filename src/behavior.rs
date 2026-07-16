use std::future::Future;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NoEvidence;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceChain<A, B> {
    pub previous: A,
    pub current: B,
}

impl<A, B> EvidenceChain<A, B> {
    pub fn new(previous: A, current: B) -> Self {
        Self { previous, current }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Offered<I, O> {
    pub(crate) input: I,
    pub(crate) offers: O,
}

impl<I, O> Offered<I, O> {
    pub fn input(&self) -> &I {
        &self.input
    }

    pub fn offers(&self) -> &O {
        &self.offers
    }

    pub fn into_parts(self) -> (I, O) {
        (self.input, self.offers)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Chosen<I, S> {
    pub(crate) input: I,
    pub(crate) source: S,
}

impl<I, S> Chosen<I, S> {
    #[allow(dead_code)]
    pub(crate) fn from_selected(input: I, source: S) -> Self {
        Self { input, source }
    }

    pub fn input(&self) -> &I {
        &self.input
    }

    pub fn source(&self) -> &S {
        &self.source
    }

    pub fn into_parts(self) -> (I, S) {
        (self.input, self.source)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Acquired<I, M, E = NoEvidence> {
    pub(crate) input: I,
    pub(crate) material: M,
    pub(crate) evidence: E,
}

impl<I, M, E> Acquired<I, M, E> {
    #[allow(dead_code)]
    pub(crate) fn from_acquire(input: I, material: M, evidence: E) -> Self {
        Self {
            input,
            material,
            evidence,
        }
    }

    pub fn input(&self) -> &I {
        &self.input
    }

    pub fn material(&self) -> &M {
        &self.material
    }

    pub fn evidence(&self) -> &E {
        &self.evidence
    }

    pub fn into_parts(self) -> (I, M, E) {
        (self.input, self.material, self.evidence)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Verified<I, M, E = NoEvidence> {
    pub(crate) input: I,
    pub(crate) material: M,
    pub(crate) evidence: E,
}

impl<I, M, E> Verified<I, M, E> {
    #[allow(dead_code)]
    pub(crate) fn from_verify(input: I, material: M, evidence: E) -> Self {
        Self {
            input,
            material,
            evidence,
        }
    }

    pub fn input(&self) -> &I {
        &self.input
    }

    pub fn material(&self) -> &M {
        &self.material
    }

    pub fn evidence(&self) -> &E {
        &self.evidence
    }

    pub fn into_parts(self) -> (I, M, E) {
        (self.input, self.material, self.evidence)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Prepared<I, P, E = NoEvidence> {
    pub(crate) input: I,
    pub(crate) prepared: P,
    pub(crate) evidence: E,
}

impl<I, P, E> Prepared<I, P, E> {
    #[allow(dead_code)]
    pub(crate) fn from_prepare(input: I, prepared: P, evidence: E) -> Self {
        Self {
            input,
            prepared,
            evidence,
        }
    }

    pub fn input(&self) -> &I {
        &self.input
    }

    pub fn prepared(&self) -> &P {
        &self.prepared
    }

    pub fn evidence(&self) -> &E {
        &self.evidence
    }

    pub fn into_parts(self) -> (I, P, E) {
        (self.input, self.prepared, self.evidence)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Applied<I, R, E = NoEvidence> {
    pub(crate) input: I,
    pub(crate) receipt: R,
    pub(crate) evidence: E,
}

impl<I, R, E> Applied<I, R, E> {
    #[allow(dead_code)]
    pub(crate) fn from_apply(input: I, receipt: R, evidence: E) -> Self {
        Self {
            input,
            receipt,
            evidence,
        }
    }

    pub fn input(&self) -> &I {
        &self.input
    }

    pub fn receipt(&self) -> &R {
        &self.receipt
    }

    pub fn evidence(&self) -> &E {
        &self.evidence
    }

    pub fn into_parts(self) -> (I, R, E) {
        (self.input, self.receipt, self.evidence)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Remembered<I, R, E = NoEvidence> {
    pub(crate) input: I,
    pub(crate) receipt: R,
    pub(crate) evidence: E,
}

impl<I, R, E> Remembered<I, R, E> {
    #[allow(dead_code)]
    pub(crate) fn from_remember(input: I, receipt: R, evidence: E) -> Self {
        Self {
            input,
            receipt,
            evidence,
        }
    }

    pub fn input(&self) -> &I {
        &self.input
    }

    pub fn receipt(&self) -> &R {
        &self.receipt
    }

    pub fn evidence(&self) -> &E {
        &self.evidence
    }

    pub fn into_parts(self) -> (I, R, E) {
        (self.input, self.receipt, self.evidence)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Observed<I, R, E = NoEvidence> {
    pub(crate) input: I,
    pub(crate) receipt: R,
    pub(crate) evidence: E,
}

impl<I, R, E> Observed<I, R, E> {
    pub fn input(&self) -> &I {
        &self.input
    }

    pub fn receipt(&self) -> &R {
        &self.receipt
    }

    pub fn evidence(&self) -> &E {
        &self.evidence
    }

    pub fn into_parts(self) -> (I, R, E) {
        (self.input, self.receipt, self.evidence)
    }
}

pub trait OfferNode<N> {
    type Offers;
    type Error;
    type Output;

    fn offer_node(&self, node: N) -> Result<Self::Output, Self::Error>;
}

pub trait SelectNode<N> {
    type Source;
    type Error;
    type Output;

    fn select_node(&self, node: N) -> Result<Self::Output, Self::Error>;
}

pub trait AcquireNode<N> {
    type Material;
    type Evidence;
    type Error;
    type Output;

    fn acquire_node(&self, node: N) -> Result<Self::Output, Self::Error>;
}

pub trait AsyncAcquireNode<N> {
    type Material;
    type Evidence;
    type Error;
    type Output;
    type Future<'a>: Future<Output = Result<Self::Output, Self::Error>>
    where
        Self: 'a,
        N: 'a;

    fn acquire_node_async(&self, node: N) -> Self::Future<'_>;
}

pub trait VerifyNode<N> {
    type Need;
    type Evidence;
    type Error;
    type Output;

    fn verify_node(&self, node: N, need: Self::Need) -> Result<Self::Output, Self::Error>;
}

pub trait AsyncVerifyNode<N> {
    type Need;
    type Evidence;
    type Error;
    type Output;
    type Future<'a>: Future<Output = Result<Self::Output, Self::Error>>
    where
        Self: 'a,
        N: 'a;

    fn verify_node_async(&self, node: N, need: Self::Need) -> Self::Future<'_>;
}

pub trait PrepareNode<N> {
    type Need;
    type Prepared;
    type Evidence;
    type Error;
    type Output;

    fn prepare_node(&self, node: N, need: Self::Need) -> Result<Self::Output, Self::Error>;
}

pub trait AsyncPrepareNode<N> {
    type Need;
    type Prepared;
    type Evidence;
    type Error;
    type Output;
    type Future<'a>: Future<Output = Result<Self::Output, Self::Error>>
    where
        Self: 'a,
        N: 'a;

    fn prepare_node_async(&self, node: N, need: Self::Need) -> Self::Future<'_>;
}

pub trait ApplyNode<N> {
    type Receipt;
    type Evidence;
    type Error;
    type Output;

    fn apply_node(&self, node: N) -> Result<Self::Output, Self::Error>;
}

pub trait AsyncApplyNode<N> {
    type Receipt;
    type Evidence;
    type Error;
    type Output;
    type Future<'a>: Future<Output = Result<Self::Output, Self::Error>>
    where
        Self: 'a,
        N: 'a;

    fn apply_node_async(&self, node: N) -> Self::Future<'_>;
}

pub trait RememberNode<N> {
    type Evidence;
    type Error;
    type Output;

    fn remember_node(&self, node: N) -> Result<Self::Output, Self::Error>;
}

pub trait AsyncRememberNode<N> {
    type Evidence;
    type Error;
    type Output;
    type Future<'a>: Future<Output = Result<Self::Output, Self::Error>>
    where
        Self: 'a,
        N: 'a;

    fn remember_node_async(&self, node: N) -> Self::Future<'_>;
}
