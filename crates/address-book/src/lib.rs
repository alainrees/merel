//! The computer's own address book, to find who in it is on Signal, as Signal on the phone does
//! with the phone's. macOS: Contacten, through the Contacts framework; the system asks once
//! whether Merel may read it (the reason is in the app's Info.plist). Only names and phone
//! numbers are read. Elsewhere there is none, and Merel's own contacts are used.

/// The people in the address book that have a phone number: their name and numbers, as typed.
/// Empty when there is none, or Merel may not read it. Can wait for the person to answer the
/// system's question, so call it off the main thread.
pub fn read() -> Vec<(String, Vec<String>)> {
    #[cfg(target_os = "macos")]
    {
        mac::read()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::cell::RefCell;
    use std::ptr::NonNull;

    use block2::{RcBlock, StackBlock};
    use objc2::AllocAnyThread as _;
    use objc2::runtime::{Bool, ProtocolObject};
    use objc2_contacts::{
        CNAuthorizationStatus, CNContact, CNContactFamilyNameKey, CNContactFetchRequest, CNContactGivenNameKey, CNContactOrganizationNameKey,
        CNContactPhoneNumbersKey, CNContactStore, CNEntityType, CNKeyDescriptor,
    };
    use objc2_foundation::{NSArray, NSError};

    /// Asks the person, the first time, whether Merel may read Contacten, and waits for the
    /// answer: reading before it is given finds nobody, and Merel would not look again.
    fn allowed(store: &CNContactStore) -> bool {
        // SAFETY: a class method taking the framework's own entity type.
        let status = unsafe { CNContactStore::authorizationStatusForEntityType(CNEntityType::Contacts) };
        if status == CNAuthorizationStatus::Authorized || status == CNAuthorizationStatus::Limited {
            return true;
        }
        if status != CNAuthorizationStatus::NotDetermined {
            log::warn!("address book: Merel may not read Contacten ({status:?})");
            return false;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let answer = RcBlock::new(move |granted: Bool, _error: *mut NSError| {
            let _ = tx.send(granted.as_bool());
        });
        // SAFETY: the block only sends on a channel; the framework calls it once, on a queue of
        // its own, and keeps it as long as it needs.
        unsafe { store.requestAccessForEntityType_completionHandler(CNEntityType::Contacts, &answer) };
        let granted = rx.recv_timeout(std::time::Duration::from_secs(600)).unwrap_or(false);
        if !granted {
            log::warn!("address book: Contacten not allowed");
        }
        granted
    }

    pub fn read() -> Vec<(String, Vec<String>)> {
        // SAFETY: a plain new store; it is safe to use from any thread.
        let store = unsafe { CNContactStore::new() };
        if !allowed(&store) {
            return Vec::new();
        }
        let people = RefCell::new(Vec::new());
        let each = |contact: NonNull<CNContact>, _stop: NonNull<Bool>| {
            // SAFETY: the framework hands a valid contact for the call's duration, fetched with
            // the keys read below.
            let (name, numbers) = unsafe {
                let contact = contact.as_ref();
                let name = format!("{} {}", contact.givenName(), contact.familyName());
                let name = if name.trim().is_empty() { contact.organizationName().to_string() } else { name.trim().to_string() };
                let numbers: Vec<String> = contact.phoneNumbers().iter().map(|labeled| labeled.value().stringValue().to_string()).collect();
                (name, numbers)
            };
            if !numbers.is_empty() && !name.is_empty() {
                people.borrow_mut().push((name, numbers));
            }
        };
        let block = StackBlock::new(each);
        let mut error: Option<objc2::rc::Retained<NSError>> = None;
        // SAFETY: the keys are the framework's own constants, the request asks for only those,
        // and the block reads only what they fetch. Off the main thread the call waits while the
        // system asks the person, as Apple documents.
        let read = unsafe {
            let keys = [
                ProtocolObject::<dyn CNKeyDescriptor>::from_ref(CNContactGivenNameKey),
                ProtocolObject::from_ref(CNContactFamilyNameKey),
                ProtocolObject::from_ref(CNContactOrganizationNameKey),
                ProtocolObject::from_ref(CNContactPhoneNumbersKey),
            ];
            let keys = NSArray::from_slice(&keys);
            let request = CNContactFetchRequest::initWithKeysToFetch(CNContactFetchRequest::alloc(), &keys);
            store.enumerateContactsWithFetchRequest_error_usingBlock(&request, Some(&mut error), &block)
        };
        if !read {
            log::warn!("address book: not read: {error:?}");
        }
        drop(block);
        let people = people.into_inner();
        log::info!("address book: {} people with a phone number", people.len());
        people
    }
}
