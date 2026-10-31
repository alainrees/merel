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

    use block2::StackBlock;
    use objc2::AllocAnyThread as _;
    use objc2::runtime::{Bool, ProtocolObject};
    use objc2_contacts::{
        CNContact, CNContactFamilyNameKey, CNContactFetchRequest, CNContactGivenNameKey, CNContactOrganizationNameKey, CNContactPhoneNumbersKey,
        CNContactStore, CNKeyDescriptor,
    };
    use objc2_foundation::{NSArray, NSError};

    pub fn read() -> Vec<(String, Vec<String>)> {
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
            CNContactStore::new().enumerateContactsWithFetchRequest_error_usingBlock(&request, Some(&mut error), &block)
        };
        if !read {
            log::warn!("address book: not read: {error:?}");
        }
        drop(block);
        people.into_inner()
    }
}
