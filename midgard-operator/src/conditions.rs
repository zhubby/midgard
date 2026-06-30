use k8s_openapi::apimachinery::pkg::apis::meta::v1::{Condition, Time};

pub fn set_condition(
    conditions: &mut Vec<Condition>,
    generation: i64,
    cond_type: &str,
    reason: &str,
    message: &str,
    status: &str,
) {
    let now = Time(k8s_openapi::jiff::Timestamp::now());
    if let Some(existing) = conditions
        .iter_mut()
        .find(|condition| condition.type_ == cond_type)
    {
        if existing.status != status || existing.reason != reason || existing.message != message {
            existing.last_transition_time = now;
        }
        existing.status = status.to_string();
        existing.reason = reason.to_string();
        existing.message = message.to_string();
        existing.observed_generation = Some(generation);
        return;
    }

    conditions.push(Condition {
        type_: cond_type.to_string(),
        status: status.to_string(),
        reason: reason.to_string(),
        message: message.to_string(),
        observed_generation: Some(generation),
        last_transition_time: now,
    });
}

pub fn remove_condition(conditions: &mut Vec<Condition>, cond_type: &str) {
    conditions.retain(|condition| condition.type_ != cond_type);
}

pub fn remove_condition_if_reason(conditions: &mut Vec<Condition>, cond_type: &str, reason: &str) {
    conditions.retain(|condition| !(condition.type_ == cond_type && condition.reason == reason));
}

pub fn find_condition<'a>(conditions: &'a [Condition], cond_type: &str) -> Option<&'a Condition> {
    conditions
        .iter()
        .find(|condition| condition.type_ == cond_type)
}
