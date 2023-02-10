pub mod protos {
    include!(concat!(env!("OUT_DIR"), "/mod.rs"));
}

use std::result;

use cyfs_base::*;
use cyfs_core::{
    GroupConsensusBlock, GroupConsensusBlockObject, GroupRPath, GroupRPathStatus, HotstuffBlockQC,
    HotstuffTimeout,
};
use cyfs_lib::NONObjectInfo;
use sha2::Digest;

#[derive(RawEncode, RawDecode, PartialEq, Eq, Ord, Clone, Debug)]
pub enum SyncBound {
    Height(u64),
    Round(u64),
}

impl Copy for SyncBound {}

impl SyncBound {
    pub fn value(&self) -> u64 {
        match self {
            Self::Height(h) => *h,
            Self::Round(r) => *r,
        }
    }

    pub fn height(&self) -> u64 {
        match self {
            Self::Height(h) => *h,
            Self::Round(r) => panic!("should be height"),
        }
    }

    pub fn round(&self) -> u64 {
        match self {
            Self::Round(r) => *r,
            Self::Height(h) => panic!("should be round"),
        }
    }

    pub fn add(&self, value: u64) -> Self {
        match self {
            Self::Height(h) => Self::Height(*h + value),
            Self::Round(r) => Self::Round(*r + value),
        }
    }

    pub fn sub(&self, value: u64) -> Self {
        match self {
            Self::Height(h) => Self::Height(*h - value),
            Self::Round(r) => Self::Round(*r - value),
        }
    }
}

impl PartialOrd for SyncBound {
    fn partial_cmp(&self, other: &SyncBound) -> Option<std::cmp::Ordering> {
        let ord = match self {
            Self::Height(height) => match other {
                Self::Height(other_height) => height.cmp(other_height),
                Self::Round(other_round) => {
                    if height >= other_round {
                        std::cmp::Ordering::Greater
                    } else {
                        std::cmp::Ordering::Less
                    }
                }
            },
            Self::Round(round) => match other {
                Self::Round(other_round) => round.cmp(other_round),
                Self::Height(other_height) => {
                    if other_height >= round {
                        std::cmp::Ordering::Less
                    } else {
                        std::cmp::Ordering::Greater
                    }
                }
            },
        };

        Some(ord)
    }
}

#[derive(Clone, RawEncode, RawDecode)]
pub(crate) enum HotstuffMessage {
    Block(cyfs_core::GroupConsensusBlock),
    BlockVote(HotstuffBlockQCVote),
    TimeoutVote(HotstuffTimeoutVote),
    Timeout(cyfs_core::HotstuffTimeout),

    SyncRequest(SyncBound, SyncBound), // [min, max]

    LastStateRequest,
    StateChangeNotify(GroupConsensusBlock, GroupConsensusBlock), // (block, qc-block)
    ProposalResult(
        ObjectId,
        BuckyResult<(
            Option<NONObjectInfo>,
            GroupConsensusBlock,
            GroupConsensusBlock,
        )>,
    ), // (proposal-id, (ExecuteResult, block, qc-block))
    QueryState(String),
    VerifiableState(String, BuckyResult<GroupRPathStatus>),
}

impl std::fmt::Debug for HotstuffMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Block(block) => {
                write!(
                    f,
                    "HotstuffMessage::Block({}/{})",
                    block.block_id(),
                    block.round()
                )
            }
            Self::BlockVote(vote) => {
                write!(
                    f,
                    "HotstuffMessage::BlockVote({}/{})",
                    vote.block_id, vote.round
                )
            }
            Self::TimeoutVote(vote) => {
                write!(
                    f,
                    "HotstuffMessage::TimeoutVote({}/{})",
                    vote.round, vote.voter
                )
            }
            Self::Timeout(tc) => {
                write!(
                    f,
                    "HotstuffMessage::Timeout({}/{:?})",
                    tc.round,
                    tc.votes.iter().map(|v| v.voter).collect::<Vec<_>>()
                )
            }
            Self::SyncRequest(min, max) => {
                write!(f, "HotstuffMessage::SyncRequest([{:?}-{:?}])", min, max)
            }
            Self::StateChangeNotify(block, qc) => {
                write!(
                    f,
                    "HotstuffMessage::StateChangeNotify({}/{}, {}/{})",
                    block.block_id(),
                    block.round(),
                    qc.block_id(),
                    qc.round()
                )
            }
            Self::LastStateRequest => {
                write!(f, "HotstuffMessage::LastStateRequest",)
            }
            Self::ProposalResult(proposal_id, result) => {
                write!(
                    f,
                    "HotstuffMessage::ProposalResult({}, {:?})",
                    proposal_id,
                    result.as_ref().map_or_else(
                        |err| { Err(err) },
                        |(obj, block, qc)| {
                            let ok = format!(
                                "({:?}, {}/{}, {}/{})",
                                obj.as_ref().map(|o| o.object_id),
                                block.block_id(),
                                block.round(),
                                qc.block_id(),
                                qc.round()
                            );
                            Ok(ok)
                        }
                    )
                )
            }
            Self::QueryState(sub_path) => {
                write!(f, "HotstuffMessage::QueryState({})", sub_path)
            }
            Self::VerifiableState(sub_path, result) => {
                write!(
                    f,
                    "HotstuffMessage::VerifiableState({}, {:?})",
                    sub_path,
                    result.as_ref().map(|status| unimplemented!())
                )
            }
        }
    }
}

const PACKAGE_FLAG_BITS: usize = 1;
const PACKAGE_FLAG_PROPOSAL_RESULT_OK: u8 = 0x80u8;

#[derive(Clone)]
pub(crate) enum HotstuffPackage {
    Block(cyfs_core::GroupConsensusBlock),
    BlockVote(ProtocolAddress, HotstuffBlockQCVote),
    TimeoutVote(ProtocolAddress, HotstuffTimeoutVote),
    Timeout(ProtocolAddress, cyfs_core::HotstuffTimeout),

    SyncRequest(ProtocolAddress, SyncBound, SyncBound),

    StateChangeNotify(GroupConsensusBlock, GroupConsensusBlock), // (block, qc-block)
    LastStateRequest(ProtocolAddress),
    ProposalResult(
        ObjectId,
        Result<
            (
                Option<NONObjectInfo>,
                GroupConsensusBlock,
                GroupConsensusBlock,
            ),
            (BuckyError, ProtocolAddress),
        >,
    ), // (proposal-id, ExecuteResult)
    QueryState(ProtocolAddress, String),
    VerifiableState(ProtocolAddress, String, BuckyResult<GroupRPathStatus>),
}

impl std::fmt::Debug for HotstuffPackage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Block(block) => {
                write!(
                    f,
                    "HotstuffPackage::Block({}/{})",
                    block.block_id(),
                    block.round()
                )
            }
            Self::BlockVote(_, vote) => {
                write!(
                    f,
                    "HotstuffPackage::BlockVote({}/{})",
                    vote.block_id, vote.round
                )
            }
            Self::TimeoutVote(_, vote) => {
                write!(
                    f,
                    "HotstuffPackage::TimeoutVote({}/{})",
                    vote.round, vote.voter
                )
            }
            Self::Timeout(_, tc) => {
                write!(
                    f,
                    "HotstuffPackage::Timeout({}/{:?})",
                    tc.round,
                    tc.votes.iter().map(|v| v.voter).collect::<Vec<_>>()
                )
            }
            Self::SyncRequest(_, min, max) => {
                write!(f, "HotstuffPackage::SyncRequest([{:?}-{:?}])", min, max)
            }
            Self::StateChangeNotify(block, qc) => {
                write!(
                    f,
                    "HotstuffPackage::StateChangeNotify({}/{}, {}/{})",
                    block.block_id(),
                    block.round(),
                    qc.block_id(),
                    qc.round()
                )
            }
            Self::LastStateRequest(_) => {
                write!(f, "HotstuffPackage::LastStateRequest",)
            }
            Self::ProposalResult(proposal_id, result) => {
                write!(
                    f,
                    "HotstuffPackage::ProposalResult({}, {:?})",
                    proposal_id,
                    result.as_ref().map_or_else(
                        |(err, _)| { Err(err) },
                        |(obj, block, qc)| {
                            let ok = format!(
                                "({:?}, {}/{}, {}/{})",
                                obj.as_ref().map(|o| o.object_id),
                                block.block_id(),
                                block.round(),
                                qc.block_id(),
                                qc.round()
                            );
                            Ok(ok)
                        }
                    )
                )
            }
            Self::QueryState(_, sub_path) => {
                write!(f, "HotstuffPackage::QueryState({})", sub_path)
            }
            Self::VerifiableState(_, sub_path, result) => {
                write!(
                    f,
                    "HotstuffPackage::VerifiableState({}, {:?})",
                    sub_path,
                    result.as_ref().map(|status| unimplemented!())
                )
            }
        }
    }
}

impl HotstuffPackage {
    pub(crate) fn rpath(&self) -> &GroupRPath {
        match self {
            HotstuffPackage::Block(block) => block.r_path(),
            HotstuffPackage::BlockVote(addr, _) => addr.check_rpath(),
            HotstuffPackage::TimeoutVote(addr, _) => addr.check_rpath(),
            HotstuffPackage::Timeout(addr, _) => addr.check_rpath(),
            HotstuffPackage::SyncRequest(addr, _, _) => addr.check_rpath(),
            HotstuffPackage::StateChangeNotify(block, _) => block.r_path(),
            HotstuffPackage::LastStateRequest(addr) => addr.check_rpath(),
            HotstuffPackage::ProposalResult(_, result) => result.as_ref().map_or_else(
                |(_, addr)| addr.check_rpath(),
                |(_, block, _)| block.r_path(),
            ),
            HotstuffPackage::QueryState(addr, _) => addr.check_rpath(),
            HotstuffPackage::VerifiableState(addr, _, _) => addr.check_rpath(),
        }
    }
}

fn encode_with_length<'a, O: RawEncode>(
    buf: &'a mut [u8],
    obj: &O,
    purpose: &Option<RawEncodePurpose>,
    length_size: usize,
) -> BuckyResult<&'a mut [u8]> {
    let (len_buf, buf) = buf.split_at_mut(length_size);
    let before_len = buf.len();
    let buf = obj.raw_encode(buf, purpose)?;
    let len = before_len - buf.len();
    assert!(len <= (1 << (length_size << 3)) - 1);
    len_buf.copy_from_slice(&len.to_le_bytes()[..length_size]);

    Ok(buf)
}

fn decode_with_length<'de, O: RawDecode<'de>>(
    buf: &'de [u8],
    length_size: usize,
) -> BuckyResult<(O, &'de [u8])> {
    assert!(length_size <= 4);
    let (len_buf, buf) = buf.split_at(length_size);

    let mut len_buf_4 = [0u8; 4];
    len_buf_4[..length_size].copy_from_slice(len_buf);
    let len = u32::from_le_bytes(len_buf_4) as usize;

    let before_len = buf.len();
    let (obj, remain) = O::raw_decode(&buf[..len])?;
    assert_eq!(remain.len(), 0);

    Ok((obj, &buf[len..]))
}

impl RawEncode for HotstuffPackage {
    fn raw_measure(&self, purpose: &Option<RawEncodePurpose>) -> BuckyResult<usize> {
        let len = match self {
            HotstuffPackage::Block(b) => b.raw_measure(purpose)?,
            HotstuffPackage::BlockVote(addr, vote) => {
                2 + addr.raw_measure(purpose)? + vote.raw_measure(purpose)?
            }
            HotstuffPackage::TimeoutVote(addr, vote) => {
                2 + addr.raw_measure(purpose)? + vote.raw_measure(purpose)?
            }
            HotstuffPackage::Timeout(addr, tc) => {
                2 + addr.raw_measure(purpose)? + tc.raw_measure(purpose)?
            }
            HotstuffPackage::SyncRequest(addr, min, max) => {
                addr.raw_measure(purpose)? + min.raw_measure(purpose)? + max.raw_measure(purpose)?
            }
            HotstuffPackage::StateChangeNotify(block, qc) => {
                3 + block.raw_measure(purpose)? + qc.raw_measure(purpose)?
            }
            HotstuffPackage::LastStateRequest(addr) => addr.raw_measure(purpose)?,
            HotstuffPackage::ProposalResult(id, result) => {
                id.raw_measure(purpose)?
                    + match result {
                        Ok((non, block, block_qc)) => {
                            non.raw_measure(purpose)?
                                + 3
                                + block.raw_measure(purpose)?
                                + block_qc.raw_measure(purpose)?
                        }
                        Err((err, addr)) => {
                            err.raw_measure(purpose)? + addr.raw_measure(purpose)?
                        }
                    }
            }
            HotstuffPackage::QueryState(addr, sub_path) => {
                addr.raw_measure(purpose)? + sub_path.raw_measure(purpose)?
            }
            HotstuffPackage::VerifiableState(addr, sub_path, result) => {
                2 + addr.raw_measure(purpose)?
                    + sub_path.raw_measure(purpose)?
                    + result.raw_measure(purpose)?
            }
        };

        Ok(1 + len)
    }

    fn raw_encode<'a>(
        &self,
        buf: &'a mut [u8],
        purpose: &Option<RawEncodePurpose>,
    ) -> BuckyResult<&'a mut [u8]> {
        match self {
            HotstuffPackage::Block(b) => {
                buf[0] = 0;
                let buf = &mut buf[1..];
                b.raw_encode(buf, purpose)
            }
            HotstuffPackage::BlockVote(addr, vote) => {
                buf[0] = 1;
                let buf = &mut buf[1..];
                let buf = encode_with_length(buf, addr, purpose, 2)?;
                vote.raw_encode(buf, purpose)
            }
            HotstuffPackage::TimeoutVote(addr, vote) => {
                buf[0] = 2;
                let buf = &mut buf[1..];
                let buf = encode_with_length(buf, addr, purpose, 2)?;
                vote.raw_encode(buf, purpose)
            }
            HotstuffPackage::Timeout(addr, tc) => {
                buf[0] = 3;
                let buf = &mut buf[1..];
                let buf = encode_with_length(buf, addr, purpose, 2)?;
                tc.raw_encode(buf, purpose)
            }
            HotstuffPackage::SyncRequest(addr, min, max) => {
                buf[0] = 4;
                let buf = &mut buf[1..];
                let buf = min.raw_encode(buf, purpose)?;
                let buf = max.raw_encode(buf, purpose)?;
                addr.raw_encode(buf, purpose)
            }
            HotstuffPackage::StateChangeNotify(block, qc) => {
                buf[0] = 5;
                let buf = &mut buf[1..];
                let buf = encode_with_length(buf, block, purpose, 3)?;
                qc.raw_encode(buf, purpose)
            }
            HotstuffPackage::LastStateRequest(addr) => {
                buf[0] = 6;
                let buf = &mut buf[1..];
                addr.raw_encode(buf, purpose)
            }
            HotstuffPackage::ProposalResult(id, result) => {
                buf[0] = 7;
                if result.is_ok() {
                    buf[0] &= PACKAGE_FLAG_PROPOSAL_RESULT_OK;
                }

                let buf = &mut buf[1..];
                let buf = id.raw_encode(buf, purpose)?;
                match result {
                    Ok((non, block, qc)) => {
                        let buf = non.raw_encode(buf, purpose)?;
                        let buf = encode_with_length(buf, block, purpose, 3)?;
                        qc.raw_encode(buf, purpose)
                    }
                    Err((err, addr)) => {
                        let buf = err.raw_encode(buf, purpose)?;
                        addr.raw_encode(buf, purpose)
                    }
                }
            }
            HotstuffPackage::QueryState(addr, sub_path) => {
                buf[0] = 8;
                let buf = &mut buf[1..];
                let buf = sub_path.raw_encode(buf, purpose)?;
                addr.raw_encode(buf, purpose)
            }
            HotstuffPackage::VerifiableState(addr, sub_path, result) => {
                buf[0] = 9;
                let buf = &mut buf[1..];
                let buf = encode_with_length(buf, addr, purpose, 2)?;
                let buf = sub_path.raw_encode(buf, purpose)?;
                result.raw_encode(buf, purpose)
            }
        }
    }
}

impl<'de> RawDecode<'de> for HotstuffPackage {
    fn raw_decode(buf: &'de [u8]) -> BuckyResult<(Self, &'de [u8])> {
        let pkg_type = buf[0] << PACKAGE_FLAG_BITS >> PACKAGE_FLAG_BITS;
        // let pkg_flag = buf[0] - pkg_type;

        match pkg_type {
            0 => {
                let buf = &buf[1..];
                let (b, buf) = GroupConsensusBlock::raw_decode(buf)?;
                Ok((HotstuffPackage::Block(b), buf))
            }
            1 => {
                let buf = &buf[1..];
                let (addr, buf) = decode_with_length(buf, 2)?;
                let (vote, buf) = HotstuffBlockQCVote::raw_decode(buf)?;
                Ok((HotstuffPackage::BlockVote(addr, vote), buf))
            }
            2 => {
                let buf = &buf[1..];
                let (addr, buf) = decode_with_length(buf, 2)?;
                let (vote, buf) = HotstuffTimeoutVote::raw_decode(buf)?;
                Ok((HotstuffPackage::TimeoutVote(addr, vote), buf))
            }
            3 => {
                let buf = &buf[1..];
                let (addr, buf) = decode_with_length(buf, 2)?;
                let (vote, buf) = HotstuffTimeout::raw_decode(buf)?;
                Ok((HotstuffPackage::Timeout(addr, vote), buf))
            }
            4 => {
                let buf = &buf[1..];
                let (min, buf) = SyncBound::raw_decode(buf)?;
                let (max, buf) = SyncBound::raw_decode(buf)?;
                let (addr, buf) = ProtocolAddress::raw_decode(buf)?;
                Ok((HotstuffPackage::SyncRequest(addr, min, max), buf))
            }
            5 => {
                let buf = &buf[1..];
                let (block, buf) = decode_with_length(buf, 3)?;
                let (qc, buf) = GroupConsensusBlock::raw_decode(buf)?;
                Ok((HotstuffPackage::StateChangeNotify(block, qc), buf))
            }
            6 => {
                let buf = &buf[1..];
                let (addr, buf) = ProtocolAddress::raw_decode(buf)?;
                Ok((HotstuffPackage::LastStateRequest(addr), buf))
            }
            7 => {
                let is_ok = (buf[0] & PACKAGE_FLAG_PROPOSAL_RESULT_OK) != 0;
                let buf = &buf[1..];
                let (id, buf) = ObjectId::raw_decode(buf)?;
                match is_ok {
                    true => {
                        let (non, buf) = Option::<NONObjectInfo>::raw_decode(buf)?;
                        let (block, buf) = decode_with_length(buf, 3)?;
                        let (qc, buf) = GroupConsensusBlock::raw_decode(buf)?;
                        Ok((
                            HotstuffPackage::ProposalResult(id, Ok((non, block, qc))),
                            buf,
                        ))
                    }
                    false => {
                        let (err, buf) = BuckyError::raw_decode(buf)?;
                        let (addr, buf) = ProtocolAddress::raw_decode(buf)?;
                        Ok((HotstuffPackage::ProposalResult(id, Err((err, addr))), buf))
                    }
                }
            }
            8 => {
                let buf = &buf[1..];
                let (sub_path, buf) = String::raw_decode(buf)?;
                let (addr, buf) = ProtocolAddress::raw_decode(buf)?;
                Ok((HotstuffPackage::QueryState(addr, sub_path), buf))
            }
            9 => {
                let buf = &buf[1..];
                let (addr, buf) = decode_with_length(buf, 3)?;
                let (sub_path, buf) = String::raw_decode(buf)?;
                let (result, buf) = BuckyResult::<GroupRPathStatus>::raw_decode(buf)?;
                Ok((
                    HotstuffPackage::VerifiableState(addr, sub_path, result),
                    buf,
                ))
            }
            _ => unreachable!("unknown protocol"),
        }
    }
}

impl HotstuffPackage {
    pub fn from_msg(msg: HotstuffMessage, rpath: GroupRPath) -> Self {
        match msg {
            HotstuffMessage::Block(block) => HotstuffPackage::Block(block),
            HotstuffMessage::BlockVote(vote) => {
                HotstuffPackage::BlockVote(ProtocolAddress::Full(rpath), vote)
            }
            HotstuffMessage::TimeoutVote(vote) => {
                HotstuffPackage::TimeoutVote(ProtocolAddress::Full(rpath), vote)
            }
            HotstuffMessage::Timeout(tc) => {
                HotstuffPackage::Timeout(ProtocolAddress::Full(rpath), tc)
            }
            HotstuffMessage::SyncRequest(min_bound, max_bound) => {
                HotstuffPackage::SyncRequest(ProtocolAddress::Full(rpath), min_bound, max_bound)
            }
            HotstuffMessage::LastStateRequest => {
                HotstuffPackage::LastStateRequest(ProtocolAddress::Full(rpath))
            }
            HotstuffMessage::StateChangeNotify(header_block, qc_block) => {
                HotstuffPackage::StateChangeNotify(header_block, qc_block)
            }
            HotstuffMessage::ProposalResult(proposal_id, result) => {
                HotstuffPackage::ProposalResult(
                    proposal_id,
                    result.map_err(|err| (err, ProtocolAddress::Full(rpath))),
                )
            }
            HotstuffMessage::QueryState(sub_path) => {
                HotstuffPackage::QueryState(ProtocolAddress::Full(rpath), sub_path)
            }
            HotstuffMessage::VerifiableState(sub_path, result) => {
                HotstuffPackage::VerifiableState(ProtocolAddress::Full(rpath), sub_path, result)
            }
        }
    }
}

#[derive(Clone, RawEncode, RawDecode)]
pub(crate) enum ProtocolAddress {
    Full(GroupRPath),
    Channel(u64),
}

impl ProtocolAddress {
    pub fn check_rpath(&self) -> &GroupRPath {
        match self {
            ProtocolAddress::Full(rpath) => rpath,
            ProtocolAddress::Channel(_) => panic!("no rpath"),
        }
    }
}

#[derive(Clone, ProtobufEncode, ProtobufDecode, ProtobufTransformType)]
#[cyfs_protobuf_type(crate::protos::HotstuffBlockQcVote)]
pub(crate) struct HotstuffBlockQCVote {
    pub block_id: ObjectId,
    pub prev_block_id: Option<ObjectId>,
    pub round: u64,
    pub voter: ObjectId,
    pub signature: Signature,
}

impl HotstuffBlockQCVote {
    pub async fn new(
        block: &GroupConsensusBlock,
        local_device_id: ObjectId,
        signer: &RsaCPUObjectSigner,
    ) -> BuckyResult<Self> {
        let block_id = block.block_id().object_id();
        let round = block.round();
        let signature = signer
            .sign(
                Self::hash_content(block_id, block.prev_block_id(), round).as_slice(),
                &SignatureSource::Object(ObjectLink {
                    obj_id: local_device_id,
                    obj_owner: None,
                }),
            )
            .await?;

        Ok(Self {
            block_id: block_id.clone(),
            round,
            voter: local_device_id,
            signature,
            prev_block_id: block.prev_block_id().map(|id| id.clone()),
        })
    }

    pub fn hash(&self) -> HashValue {
        Self::hash_content(&self.block_id, self.prev_block_id.as_ref(), self.round)
    }

    fn hash_content(
        block_id: &ObjectId,
        prev_block_id: Option<&ObjectId>,
        round: u64,
    ) -> HashValue {
        let mut sha256 = sha2::Sha256::new();
        sha256.input(block_id.as_slice());
        sha256.input(round.to_le_bytes());
        if let Some(prev_block_id) = prev_block_id {
            sha256.input(prev_block_id.as_slice());
        }
        sha256.result().into()
    }
}

impl ProtobufTransform<crate::protos::HotstuffBlockQcVote> for HotstuffBlockQCVote {
    fn transform(value: crate::protos::HotstuffBlockQcVote) -> BuckyResult<Self> {
        Ok(Self {
            voter: ObjectId::raw_decode(value.voter.as_slice())?.0,
            signature: Signature::raw_decode(value.signature.as_slice())?.0,
            block_id: ObjectId::raw_decode(value.block_id.as_slice())?.0,
            round: value.round,
            prev_block_id: match value.prev_block_id.as_ref() {
                Some(id) => Some(ObjectId::raw_decode(id.as_slice())?.0),
                None => None,
            },
        })
    }
}

impl ProtobufTransform<&HotstuffBlockQCVote> for crate::protos::HotstuffBlockQcVote {
    fn transform(value: &HotstuffBlockQCVote) -> BuckyResult<Self> {
        let ret = crate::protos::HotstuffBlockQcVote {
            block_id: value.block_id.to_vec()?,
            round: value.round,
            voter: value.voter.to_vec()?,
            signature: value.signature.to_vec()?,
            prev_block_id: match value.prev_block_id.as_ref() {
                Some(id) => Some(id.to_vec()?),
                None => None,
            },
        };

        Ok(ret)
    }
}

#[derive(Clone, ProtobufEncode, ProtobufDecode, ProtobufTransformType)]
#[cyfs_protobuf_type(crate::protos::HotstuffTimeoutVote)]
pub(crate) struct HotstuffTimeoutVote {
    pub high_qc: Option<HotstuffBlockQC>,
    pub round: u64,
    pub voter: ObjectId,
    pub signature: Signature,
}

impl HotstuffTimeoutVote {
    pub async fn new(
        high_qc: Option<HotstuffBlockQC>,
        round: u64,
        local_device_id: ObjectId,
        signer: &RsaCPUObjectSigner,
    ) -> BuckyResult<Self> {
        let signature = signer
            .sign(
                Self::hash_content(high_qc.as_ref().map_or(0, |qc| qc.round), round).as_slice(),
                &SignatureSource::Object(ObjectLink {
                    obj_id: local_device_id,
                    obj_owner: None,
                }),
            )
            .await?;

        Ok(Self {
            high_qc,
            round,
            voter: local_device_id,
            signature,
        })
    }

    pub fn hash(&self) -> HashValue {
        Self::hash_content(self.high_qc.as_ref().map_or(0, |qc| qc.round), self.round)
    }

    pub fn hash_content(high_qc_round: u64, round: u64) -> HashValue {
        let mut sha256 = sha2::Sha256::new();
        sha256.input(high_qc_round.to_le_bytes());
        sha256.input(round.to_le_bytes());
        sha256.result().into()
    }
}

impl ProtobufTransform<crate::protos::HotstuffTimeoutVote> for HotstuffTimeoutVote {
    fn transform(value: crate::protos::HotstuffTimeoutVote) -> BuckyResult<Self> {
        let high_qc = if value.high_qc().len() == 0 {
            None
        } else {
            Some(HotstuffBlockQC::raw_decode(value.high_qc())?.0)
        };
        Ok(Self {
            voter: ObjectId::raw_decode(value.voter.as_slice())?.0,
            signature: Signature::raw_decode(value.signature.as_slice())?.0,
            round: value.round,
            high_qc,
        })
    }
}

impl ProtobufTransform<&HotstuffTimeoutVote> for crate::protos::HotstuffTimeoutVote {
    fn transform(value: &HotstuffTimeoutVote) -> BuckyResult<Self> {
        let ret = crate::protos::HotstuffTimeoutVote {
            high_qc: match value.high_qc.as_ref() {
                Some(qc) => Some(qc.to_vec()?),
                None => None,
            },
            round: value.round,
            voter: value.voter.to_vec()?,
            signature: value.signature.to_vec()?,
        };

        Ok(ret)
    }
}

#[cfg(test)]
mod test {}
