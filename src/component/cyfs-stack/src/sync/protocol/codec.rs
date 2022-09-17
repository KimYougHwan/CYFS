use super::request::*;
use cyfs_base::*;

use serde_json::{Map, Value};

// device ping
impl JsonCodec<SyncPingRequest> for SyncPingRequest {
    fn encode_json(&self) -> Map<String, Value> {
        let mut obj = Map::new();

        JsonCodecHelper::encode_string_field(&mut obj, "device_id", &self.device_id);
        JsonCodecHelper::encode_string_field(&mut obj, "zone_role", &self.zone_role);
        JsonCodecHelper::encode_string_field(&mut obj, "root_state", &self.root_state);
        JsonCodecHelper::encode_string_field(
            &mut obj,
            "root_state_revision",
            &self.root_state_revision,
        );
        JsonCodecHelper::encode_string_field(&mut obj, "state", &self.state);
        JsonCodecHelper::encode_string_field(
            &mut obj,
            "owner_update_time",
            &self.owner_update_time,
        );

        obj
    }

    fn decode_json(obj: &Map<String, Value>) -> BuckyResult<Self> {
        Ok(Self {
            device_id: JsonCodecHelper::decode_string_field(obj, "device_id")?,
            zone_role: JsonCodecHelper::decode_string_field(obj, "zone_role")?,
            root_state: JsonCodecHelper::decode_string_field(obj, "root_state")?,
            root_state_revision: JsonCodecHelper::decode_string_field(obj, "root_state_revision")?,
            state: JsonCodecHelper::decode_string_field(obj, "state")?,
            owner_update_time: JsonCodecHelper::decode_option_int_field(obj, "owner_update_time")?
                .unwrap_or(0),
        })
    }
}

impl JsonCodec<SyncPingResponse> for SyncPingResponse {
    fn encode_json(&self) -> Map<String, Value> {
        let mut obj = Map::new();

        JsonCodecHelper::encode_string_field(&mut obj, "zone_root_state", &self.zone_root_state);
        JsonCodecHelper::encode_string_field(
            &mut obj,
            "zone_root_state_revision",
            &self.zone_root_state_revision,
        );
        JsonCodecHelper::encode_string_field(&mut obj, "zone_role", &self.zone_role);
        JsonCodecHelper::encode_string_field(&mut obj, "ood_work_mode", &self.ood_work_mode);

        let owner = self
            .owner
            .as_ref()
            .map(|object_raw| hex::encode(&object_raw));

        JsonCodecHelper::encode_option_string_field(&mut obj, "owner", owner.as_ref());

        obj
    }

    fn decode_json(obj: &Map<String, Value>) -> BuckyResult<Self> {
        let owner: Option<String> = JsonCodecHelper::decode_option_string_field(obj, "owner")?;
        let owner = match owner {
            Some(s) => {
                let v = hex::decode(&s).map_err(|e| {
                    let msg = format!("decode owner from object_raw string error! s={}, {}", s, e);
                    error!("{}", msg);
                    BuckyError::new(BuckyErrorCode::InvalidFormat, msg)
                })?;
                Some(v)
            }
            None => None,
        };

        Ok(Self {
            zone_root_state: JsonCodecHelper::decode_string_field(obj, "zone_root_state")?,
            zone_root_state_revision: JsonCodecHelper::decode_string_field(
                obj,
                "zone_root_state_revision",
            )?,
            zone_role: JsonCodecHelper::decode_string_field(obj, "zone_role")?,
            ood_work_mode: JsonCodecHelper::decode_string_field(obj, "ood_work_mode")?,
            owner,
        })
    }
}

// sync-diff
impl JsonCodec<Self> for SyncDiffRequest {
    fn encode_json(&self) -> Map<String, Value> {
        let mut obj = Map::new();
        obj.insert(
            "category".to_owned(),
            Value::String(self.category.to_string()),
        );
        obj.insert("path".to_owned(), Value::String(self.path.clone()));

        JsonCodecHelper::encode_option_string_field(&mut obj, "dec_id", self.dec_id.as_ref());
        JsonCodecHelper::encode_option_string_field(&mut obj, "current", self.current.as_ref());

        obj
    }

    fn decode_json(obj: &Map<String, Value>) -> BuckyResult<Self> {
        Ok(Self {
            category: JsonCodecHelper::decode_string_field(obj, "category")?,
            path: JsonCodecHelper::decode_string_field(obj, "path")?,
            dec_id: JsonCodecHelper::decode_option_string_field(obj, "dec_id")?,
            current: JsonCodecHelper::decode_option_string_field(obj, "current")?,
        })
    }
}

impl JsonCodec<SyncObjectsRequest> for SyncObjectsRequest {
    fn encode_json(&self) -> Map<String, Value> {
        let mut obj = Map::new();

        obj.insert(
            "begin_seq".to_owned(),
            Value::String(self.begin_seq.to_string()),
        );
        obj.insert(
            "end_seq".to_owned(),
            Value::String(self.end_seq.to_string()),
        );

        obj.insert(
            "list".to_owned(),
            JsonCodecHelper::encode_to_str_array(&self.list),
        );

        obj
    }

    fn decode_json(obj: &Map<String, Value>) -> BuckyResult<Self> {
        let mut begin_seq: Option<u64> = None;
        let mut end_seq: Option<u64> = None;
        let mut list: Option<Vec<ObjectId>> = None;

        for (k, v) in obj {
            match k.as_str() {
                "begin_seq" => {
                    begin_seq = Some(JsonCodecHelper::decode_from_string(v)?);
                }
                "end_seq" => {
                    end_seq = Some(JsonCodecHelper::decode_from_string(v)?);
                }

                "list" => {
                    list = Some(JsonCodecHelper::decode_from_str_array(v)?);
                }

                u @ _ => {
                    warn!("unknown sync objects response field: {}", u);
                }
            }
        }

        if begin_seq.is_none() || end_seq.is_none() || list.is_none() {
            let msg = format!(
                "invalid sync objects response, begin_seq/end_seq/list missing: {:?}",
                obj
            );
            error!("{}", msg);

            return Err(BuckyError::new(BuckyErrorCode::InvalidFormat, msg));
        }

        Ok(Self {
            begin_seq: begin_seq.unwrap(),
            end_seq: end_seq.unwrap(),
            list: list.unwrap(),
        })
    }
}

/*
impl JsonCodec<Self> for SyncChunksRequest {
    fn encode_json(&self) -> Map<String, Value> {
        let mut obj = Map::new();

        JsonCodecHelper::encode_str_array_field(&mut obj, "list", self.list.as_ref());

        let state: Vec<u8> = self.state.iter().map(|state| u8::from(state)).collect();
        JsonCodecHelper::encode_number_array_field(&mut obj, "state", state.as_ref());

        obj
    }

    fn decode_json(obj: &Map<String, Value>) -> BuckyResult<Self> {
        let values: Vec<u8> = JsonCodecHelper::decode_int_array_field(obj, "state")?;
        let mut states = Vec::with_capacity(values.len());
        for v in values {
            let state = ChunkState::try_from(v)?;
            states.push(state);
        }

        Ok(Self {
            list: JsonCodecHelper::decode_str_array_field(obj, "list")?,
            states,
        })
    }
}
*/
